use crate::catalog::SqliteCatalog;
use bdm_domain::ingest::{valid_minimum_short_side, IngestJob, SourcePost, StoredMedia};
use bdm_ports::{IngestError, IngestRepository, MediaProvider};
use image::ImageReader;
use md5::Md5;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{BufReader, Cursor, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub(crate) fn storage<T, E: std::fmt::Debug + 'static>(
    result: Result<T, E>,
) -> Result<T, IngestError> {
    result.map_err(|error| {
        let value = &error as &dyn std::any::Any;
        if let Some(rusqlite::Error::SqliteFailure(code, _)) =
            value.downcast_ref::<rusqlite::Error>()
        {
            if matches!(
                code.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) {
                return IngestError::DatabaseBusy;
            }
            return IngestError::StorageDetails(format!(
                "SQLite {:?}/{}",
                code.code, code.extended_code
            ));
        }
        if let Some(error) = value.downcast_ref::<std::io::Error>() {
            return IngestError::StorageDetails(format!(
                "文件 {:?}，系统码 {:?}",
                error.kind(),
                error.raw_os_error()
            ));
        }
        IngestError::Storage
    })
}

pub(crate) fn validate(
    bytes: &[u8],
    post: &SourcePost,
    minimum_short_side: u32,
) -> Result<image::DynamicImage, IngestError> {
    let format = image::guess_format(bytes).map_err(|_| IngestError::InvalidMedia)?;
    let expected = match post.file_ext.as_str() {
        "png" => image::ImageFormat::Png,
        "jpg" | "jpeg" => image::ImageFormat::Jpeg,
        "webp" => image::ImageFormat::WebP,
        _ => return Err(IngestError::InvalidMedia),
    };
    if format != expected {
        return Err(IngestError::InvalidMedia);
    }
    match format {
        image::ImageFormat::Png => {
            let decoder = image::codecs::png::PngDecoder::new(Cursor::new(bytes))
                .map_err(|_| IngestError::InvalidMedia)?;
            if decoder.is_apng().map_err(|_| IngestError::InvalidMedia)? {
                return Err(IngestError::InvalidMedia);
            }
        }
        image::ImageFormat::WebP => {
            let decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes))
                .map_err(|_| IngestError::InvalidMedia)?;
            if decoder.has_animation() {
                return Err(IngestError::InvalidMedia);
            }
        }
        _ => {}
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| IngestError::InvalidMedia)?;
    if decoded.width() != post.image_width
        || decoded.height() != post.image_height
        || !valid_minimum_short_side(minimum_short_side)
        || decoded.width() < minimum_short_side
        || decoded.height() < minimum_short_side
    {
        return Err(IngestError::InvalidMedia);
    }
    Ok(decoded)
}

fn durable_write(path: &Path, bytes: &[u8]) -> Result<(), IngestError> {
    use std::io::Write;
    let mut file = storage(fs::File::create(path))?;
    storage(file.write_all(bytes))?;
    storage(file.sync_all())
}

pub(crate) fn has_transparency(image: &image::DynamicImage) -> bool {
    image.to_rgba8().pixels().any(|pixel| pixel[3] < 255)
}

pub(crate) fn flatten_on_white(image: &image::DynamicImage) -> image::DynamicImage {
    let rgba = image.to_rgba8();
    let rgb = image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let pixel = rgba.get_pixel(x, y).0;
        let alpha = u32::from(pixel[3]);
        image::Rgb([0, 1, 2].map(|channel| {
            ((u32::from(pixel[channel]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
        }))
    });
    image::DynamicImage::ImageRgb8(rgb)
}

impl SqliteCatalog {
    pub fn media_by_id(&self, id: i64) -> Result<Option<StoredMedia>, IngestError> {
        let row: Option<(String, bool)> = storage(
            self.connection
                .query_row("SELECT metadata,keep FROM media WHERE id=?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional(),
        )?;
        row.map(|(json, keep)| {
            let mut media: StoredMedia = storage(serde_json::from_str(&json))?;
            media.id = id;
            media.keep = keep;
            let mut query = storage(
                self.connection
                    .prepare("SELECT collection_id FROM collection_media WHERE media_id=?1"),
            )?;
            media.collections = storage(
                query
                    .query_map([id], |r| r.get(0))
                    .and_then(|rows| rows.collect()),
            )?;
            Ok(media)
        })
        .transpose()
    }
    pub fn pending_analysis(&self) -> Result<Vec<i64>, IngestError> {
        let mut query = storage(self.connection.prepare(
            "SELECT media_id FROM analysis_jobs WHERE state IN ('pending','needs_model') ORDER BY media_id DESC",
        ))?;
        storage(
            query
                .query_map([], |r| r.get(0))
                .and_then(|rows| rows.collect()),
        )
    }
    pub fn pending_analysis_batch(
        &self,
        limit: usize,
        model_available: bool,
    ) -> Result<Vec<i64>, IngestError> {
        let mut query = storage(self.connection.prepare(
            "SELECT media_id FROM analysis_jobs WHERE state='pending' OR (?1 AND state='needs_model') ORDER BY media_id LIMIT ?2",
        ))?;
        storage(
            query
                .query_map(rusqlite::params![model_available, limit as i64], |r| {
                    r.get(0)
                })
                .and_then(|rows| rows.collect()),
        )
    }
    pub fn list_media(&self) -> Result<Vec<StoredMedia>, IngestError> {
        let mut associations: std::collections::BTreeMap<i64, BTreeSet<i64>> = Default::default();
        let mut links = storage(
            self.connection
                .prepare("SELECT media_id, collection_id FROM collection_media ORDER BY media_id"),
        )?;
        for row in
            storage(links.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))))?
        {
            let (media, collection) = storage(row)?;
            associations.entry(media).or_default().insert(collection);
        }
        let mut query = storage(
            self.connection
                .prepare("SELECT id, metadata, keep FROM media ORDER BY id"),
        )?;
        let rows = storage(query.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
            ))
        }))?;
        let mut result = Vec::new();
        for row in rows {
            let (id, metadata, keep) = storage(row)?;
            let mut item: StoredMedia = storage(serde_json::from_str(&metadata))?;
            item.id = id;
            item.keep = keep;
            item.collections = associations.remove(&id).unwrap_or_default();
            result.push(item);
        }
        Ok(result)
    }

    pub fn set_media_keep(&mut self, id: i64, keep: bool) -> Result<(), IngestError> {
        if storage(
            self.connection
                .execute("UPDATE media SET keep=?2 WHERE id=?1", params![id, keep]),
        )? != 1
        {
            return Err(IngestError::Storage);
        }
        Ok(())
    }

    pub fn list_jobs(&self) -> Result<Vec<IngestJob>, IngestError> {
        let mut query = storage(
            self.connection
                .prepare("SELECT metadata FROM ingest_jobs ORDER BY id DESC"),
        )?;
        let rows = storage(query.query_map([], |row| row.get::<_, String>(0)))?;
        rows.map(|row| storage(serde_json::from_str(&storage(row)?)))
            .collect()
    }

    pub fn new_job(
        &mut self,
        collection_id: i64,
        tag: String,
        root: PathBuf,
        minimum_short_side: u32,
    ) -> Result<IngestJob, IngestError> {
        self.new_job_with_transparency(
            collection_id,
            tag,
            root,
            minimum_short_side,
            bdm_domain::ingest::default_flatten_transparency(),
        )
    }

    pub fn new_job_with_transparency(
        &mut self,
        collection_id: i64,
        tag: String,
        root: PathBuf,
        minimum_short_side: u32,
        flatten_transparency: bool,
    ) -> Result<IngestJob, IngestError> {
        if !valid_minimum_short_side(minimum_short_side) {
            return Err(IngestError::InvalidMedia);
        }
        if !root.is_absolute() {
            return Err(IngestError::Storage);
        }
        storage(fs::create_dir_all(&root))?;
        let tx = storage(
            self.connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
        )?;
        storage(tx.execute(
            "INSERT INTO ingest_jobs(collection_id, metadata) VALUES(?1,'{}')",
            [collection_id],
        ))?;
        let job = IngestJob {
            id: tx.last_insert_rowid(),
            collection_id,
            tag,
            root,
            minimum_short_side,
            flatten_transparency,
            before_id: None,
            state: "queued".into(),
            progress: Default::default(),
            error: None,
            failed_posts: Vec::new(),
        };
        storage(tx.execute(
            "UPDATE ingest_jobs SET metadata=?2 WHERE id=?1",
            params![job.id, storage(serde_json::to_string(&job))?],
        ))?;
        storage(tx.commit())?;
        Ok(job)
    }

    fn associations(
        &self,
        post: &SourcePost,
        collection: i64,
    ) -> Result<BTreeSet<i64>, IngestError> {
        let mut ids = BTreeSet::from([collection]);
        let mut query = storage(self.connection.prepare_cached(
            "SELECT id FROM collections WHERE provider='danbooru' AND tag=?1 AND category=?2",
        ))?;
        for (tags, category) in [
            (&post.tag_string_artist, 1),
            (&post.tag_string_character, 4),
        ] {
            for name in tags.split_whitespace() {
                for row in
                    storage(query.query_map(params![name, category], |row| row.get::<_, i64>(0)))?
                {
                    ids.insert(storage(row)?);
                }
            }
        }
        Ok(ids)
    }

    fn commit_media(&mut self, media: &StoredMedia) -> Result<(), IngestError> {
        let metadata = storage(serde_json::to_string(media))?;
        let source = storage(serde_json::to_string(&media.post))?;
        let tx = storage(
            self.connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
        )?;
        storage(tx.execute(
            "INSERT OR IGNORE INTO media(md5,metadata) VALUES(?1,?2)",
            params![media.post.md5, metadata],
        ))?;
        let id: i64 = storage(tx.query_row(
            "SELECT id FROM media WHERE md5=?1",
            [&media.post.md5],
            |row| row.get(0),
        ))?;
        let mut persisted = media.clone();
        persisted.id = id;
        // The original media metadata remains immutable when adding source provenance.
        if media.id == 0 {
            storage(tx.execute(
                "UPDATE media SET metadata=?2 WHERE id=?1",
                params![id, storage(serde_json::to_string(&persisted))?],
            ))?;
        }
        storage(tx.execute(
            "INSERT OR IGNORE INTO media_sources(post_id,media_id,metadata) VALUES(?1,?2,?3)",
            params![media.post.id, id, source],
        ))?;
        for collection in &media.collections {
            storage(tx.execute(
                "INSERT OR IGNORE INTO collection_media VALUES(?1,?2)",
                params![collection, id],
            ))?;
        }
        attach_post_collections(&tx, &media.post, id)?;
        storage(tx.execute(
            "INSERT OR IGNORE INTO analysis_jobs(media_id) VALUES(?1)",
            [id],
        ))?;
        storage(tx.execute("DELETE FROM import_journal WHERE md5=?1", [&media.post.md5]))?;
        storage(tx.commit())
    }

    /// Complete promoted imports; preserve damaged/uncommitted files for recovery.
    pub fn recover_imports(&mut self) -> Result<(), IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        let pending: Vec<String> = {
            let mut query = storage(
                self.connection
                    .prepare("SELECT metadata FROM import_journal"),
            )?;
            storage(
                query
                    .query_map([], |row| row.get(0))
                    .and_then(|rows| rows.collect()),
            )?
        };
        for metadata in pending {
            let media: StoredMedia = storage(serde_json::from_str(&metadata))?;
            if media.path.exists() {
                let bytes = storage(fs::read(&media.path))?;
                if format!("{:x}", Sha256::digest(&bytes)) != media.sha256
                    || validate(&bytes, &media.post, media.minimum_short_side).is_err()
                    || !media.thumbnail.exists()
                {
                    return Err(IngestError::Integrity);
                }
                self.commit_media(&media)?;
            } else {
                storage(
                    self.connection
                        .execute("DELETE FROM import_journal WHERE md5=?1", [&media.post.md5]),
                )?;
            }
        }
        // A previous process's running job requires an explicit resume action.
        for mut job in self.list_jobs()? {
            if matches!(job.state.as_str(), "running" | "queued") {
                job.state = "interrupted".into();
                self.save_job(&job)?;
            }
        }
        Ok(())
    }

    pub fn attach_known_collections(&mut self) -> Result<(), IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        let tx = storage(
            self.connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
        )?;
        {
            let mut query = storage(
                tx.prepare("SELECT media_id, metadata FROM media_sources ORDER BY media_id"),
            )?;
            for row in storage(query.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            }))? {
                let (id, json) = storage(row)?;
                let post: SourcePost = storage(serde_json::from_str(&json))?;
                attach_post_collections(&tx, &post, id)?;
            }
        }
        storage(tx.commit())
    }

    /// Finish local tags after a crash/error between media commit and tag materialization.
    pub fn recover_import_tags(&mut self) -> Result<(), IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        let ids: Vec<i64> = {
            let mut query = storage(self.connection.prepare(
                "SELECT m.id FROM media m WHERE NOT EXISTS (SELECT 1 FROM text_artifacts a WHERE a.media_id=m.id AND a.kind='tags' AND a.is_current=1)",
            ))?;
            storage(
                query
                    .query_map([], |row| row.get(0))
                    .and_then(|rows| rows.collect()),
            )?
        };
        for id in ids {
            if let Some(media) = self.media_by_id(id)? {
                self.finish_import_tags(&media)?;
            }
        }
        Ok(())
    }

    fn finish_import_tags(&mut self, media: &StoredMedia) -> Result<(), IngestError> {
        let post = &media.post;
        let mut rules = self.tag_rules()?;
        let mut changed = false;
        for (tags, category) in [
            (&post.tag_string_general, 0),
            (&post.tag_string_artist, 1),
            (&post.tag_string_copyright, 3),
            (&post.tag_string_character, 4),
            (&post.tag_string_meta, 5),
        ] {
            for name in tags.split_whitespace() {
                changed |= rules.categories.insert(name.into(), category) != Some(category);
            }
        }
        if changed {
            self.save_tag_rules(&rules)?;
        }
        self.normalize_media_locked(media)?;
        Ok(())
    }
}

/// Discover artists from authoritative post categories, without fabricating remote IDs/counts.
/// All associations and discovered records belong to the caller's media transaction.
fn attach_post_collections(
    tx: &rusqlite::Transaction<'_>,
    post: &SourcePost,
    media: i64,
) -> Result<(), IngestError> {
    let mut insert = storage(tx.prepare_cached(
        "INSERT OR IGNORE INTO collections(provider, category, tag) VALUES('danbooru', 1, ?1)",
    ))?;
    for name in post.tag_string_artist.split_whitespace() {
        storage(insert.execute([name]))?;
    }
    let mut attach = storage(tx.prepare_cached("INSERT OR IGNORE INTO collection_media(collection_id,media_id) SELECT id,?1 FROM collections WHERE provider='danbooru' AND category=?2 AND tag=?3"))?;
    for (tags, category) in [
        (&post.tag_string_artist, 1),
        (&post.tag_string_character, 4),
    ] {
        for name in tags.split_whitespace() {
            storage(attach.execute(params![media, category, name]))?;
        }
    }
    Ok(())
}

impl IngestRepository for SqliteCatalog {
    fn save_job(&mut self, job: &IngestJob) -> Result<(), IngestError> {
        let metadata = storage(serde_json::to_string(job))?;
        if storage(self.connection.execute(
            "UPDATE ingest_jobs SET metadata=?2 WHERE id=?1",
            params![job.id, metadata],
        ))? != 1
        {
            return Err(IngestError::Storage);
        }
        Ok(())
    }

    fn record_outcome(&mut self, job: i64, post: i64, outcome: &str) -> Result<(), IngestError> {
        storage(self.connection.execute(
            "INSERT OR REPLACE INTO ingest_outcomes VALUES(?1,?2,?3)",
            params![job, post, outcome],
        ))?;
        Ok(())
    }

    fn has_source(&mut self, post: &SourcePost, collection: i64) -> Result<bool, IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        self.has_source_locked(post, collection)
    }

    fn import(
        &mut self,
        post: &SourcePost,
        root: &Path,
        collection: i64,
        minimum_short_side: u32,
        cancel: &AtomicBool,
        provider: &dyn MediaProvider,
    ) -> Result<bool, IngestError> {
        self.import_post(
            post,
            root,
            collection,
            minimum_short_side,
            bdm_domain::ingest::default_flatten_transparency(),
            cancel,
            provider,
        )
    }

    fn import_with_options(
        &mut self,
        post: &SourcePost,
        root: &Path,
        collection: i64,
        minimum_short_side: u32,
        flatten_transparency: bool,
        cancel: &AtomicBool,
        provider: &dyn MediaProvider,
    ) -> Result<bool, IngestError> {
        self.import_post(
            post,
            root,
            collection,
            minimum_short_side,
            flatten_transparency,
            cancel,
            provider,
        )
    }
}

impl SqliteCatalog {
    fn has_source_locked(
        &mut self,
        post: &SourcePost,
        collection: i64,
    ) -> Result<bool, IngestError> {
        let existing: Option<(i64,String)> = storage(self.connection.query_row("SELECT m.id,s.metadata FROM media m JOIN media_sources s ON s.media_id=m.id WHERE s.post_id=?1", [post.id], |row| Ok((row.get(0)?,row.get(1)?))).optional())?;
        let Some((id, source)) = existing else {
            return Ok(false);
        };
        let source: SourcePost = storage(serde_json::from_str(&source))?;
        if source.md5 != post.md5 {
            return Err(IngestError::Integrity);
        }
        let metadata: String = storage(self.connection.query_row(
            "SELECT metadata FROM media WHERE id=?1",
            [id],
            |row| row.get(0),
        ))?;
        let media: StoredMedia = storage(serde_json::from_str(&metadata))?;
        if !media.path.is_file() {
            return Err(IngestError::Storage);
        }
        let associations = self.associations(post, collection)?;
        let tx = storage(
            self.connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
        )?;
        for collection in associations {
            storage(tx.execute(
                "INSERT OR IGNORE INTO collection_media VALUES(?1,?2)",
                params![collection, id],
            ))?;
        }
        attach_post_collections(&tx, post, id)?;
        storage(tx.commit())?;
        let tags_missing: bool = storage(self.connection.query_row(
            "SELECT NOT EXISTS (SELECT 1 FROM text_artifacts WHERE media_id=?1 AND kind='tags' AND is_current=1)",
            [id], |row| row.get(0),
        ))?;
        if tags_missing {
            self.finish_import_tags(&media)?;
        }
        Ok(true)
    }

    // Explicit source and transfer options are part of the established ingest adapter contract.
    #[allow(clippy::too_many_arguments)]
    fn import_post(
        &mut self,
        post: &SourcePost,
        root: &Path,
        collection: i64,
        minimum_short_side: u32,
        flatten_transparency: bool,
        cancel: &AtomicBool,
        provider: &dyn MediaProvider,
    ) -> Result<bool, IngestError> {
        if post.skip_reason_with_minimum(minimum_short_side).is_some() {
            return Err(IngestError::InvalidMedia);
        }
        if post.md5.len() != 32
            || !post
                .md5
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(IngestError::Integrity);
        }
        if self.has_source(post, collection)? {
            return Ok(false);
        }
        storage(fs::create_dir_all(root.join("originals")))?;
        let staging = storage(tempfile::Builder::new().prefix(".import-").tempdir_in(root))?;
        let filename = format!("original.{}", post.file_ext);
        let original = staging.path().join(&filename);
        provider.download(post, &original, cancel)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        let mut bytes = Vec::new();
        storage(
            BufReader::new(storage(fs::File::open(&original))?)
                .take(64 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes),
        )?;
        if bytes.len() as u64 != post.file_size || bytes.len() > 64 * 1024 * 1024 {
            return Err(IngestError::InvalidMedia);
        }
        if format!("{:x}", Md5::digest(&bytes)) != post.md5 {
            return Err(IngestError::Integrity);
        }
        let decoded = validate(&bytes, post, minimum_short_side)?;
        let source_sha256 = format!("{:x}", Sha256::digest(&bytes));
        let (stored_image, stored_bytes) = if flatten_transparency && has_transparency(&decoded) {
            let flattened = flatten_on_white(&decoded);
            let format = match post.file_ext.as_str() {
                "png" => image::ImageFormat::Png,
                "webp" => image::ImageFormat::WebP,
                _ => image::ImageFormat::Jpeg,
            };
            let mut encoded = Cursor::new(Vec::new());
            flattened
                .write_to(&mut encoded, format)
                .map_err(|_| IngestError::InvalidMedia)?;
            let stored_bytes = encoded.into_inner();
            durable_write(&original, &stored_bytes)?;
            (flattened, stored_bytes)
        } else {
            (decoded, bytes.clone())
        };
        // Prepare files privately before locking promotion/commit against dedup deletion.
        storage(
            stored_image
                .thumbnail(384, 384)
                .save_with_format(staging.path().join("preview.png"), image::ImageFormat::Png),
        )?;
        storage(
            fs::OpenOptions::new()
                .write(true)
                .open(staging.path().join("preview.png")),
        )?
        .sync_all()
        .map_err(|_| IngestError::Storage)?;
        durable_write(
            &staging.path().join("original.raw_tags.txt"),
            post.tag_string.as_bytes(),
        )?;
        durable_write(&staging.path().join("original.tags.txt"), b"")?;
        durable_write(&staging.path().join("original.txt"), b"")?;
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        if self.has_source_locked(post, collection)? {
            return Ok(false);
        }
        let existing: Option<String> = storage(
            self.connection
                .query_row(
                    "SELECT metadata FROM media WHERE md5=?1",
                    [&post.md5],
                    |r| r.get(0),
                )
                .optional(),
        )?;
        if let Some(json) = existing {
            let mut media: StoredMedia = storage(serde_json::from_str(&json))?;
            let expected_source = if media.source_sha256.is_empty() {
                media.sha256.as_str()
            } else {
                media.source_sha256.as_str()
            };
            if source_sha256 != expected_source {
                return Err(IngestError::Integrity);
            }
            media.post = post.clone();
            media.collections = self.associations(post, collection)?;
            self.commit_media(&media)?;
            return Ok(false);
        }
        let directory = root.join("originals").join(&post.md5);
        let media = StoredMedia {
            id: 0,
            post: post.clone(),
            path: directory.join(filename),
            thumbnail: directory.join("preview.png"),
            sha256: format!("{:x}", Sha256::digest(&stored_bytes)),
            source_sha256,
            collections: self.associations(post, collection)?,
            keep: false,
            minimum_short_side,
            flatten_transparency,
            edit: None,
        };
        if directory.exists() {
            return Err(IngestError::Storage);
        }
        storage(self.connection.execute(
            "INSERT INTO import_journal(md5,metadata) VALUES(?1,?2)",
            params![post.md5, storage(serde_json::to_string(&media))?],
        ))?;
        storage(fs::rename(staging.path(), &directory))?;
        self.commit_media(&media)?;
        let json: String = storage(self.connection.query_row(
            "SELECT metadata FROM media WHERE md5=?1",
            [&post.md5],
            |r| r.get(0),
        ))?;
        let mut imported: StoredMedia = storage(serde_json::from_str(&json))?;
        imported.id = storage(self.connection.query_row(
            "SELECT id FROM media WHERE md5=?1",
            [&post.md5],
            |r| r.get(0),
        ))?;
        self.finish_import_tags(&imported)?;
        Ok(true)
    }
}
