use crate::catalog::SqliteCatalog;
use bdm_domain::{
    catalog::{CollectionKind, SourceTag},
    ingest::SourcePost,
};
use bdm_ports::{CollectionRepository, IngestError, IngestRepository, MediaProvider};
use md5::{Digest, Md5};
use sha2::{Digest as Sha2Digest, Sha256};
use std::{cell::Cell, path::Path, sync::atomic::AtomicBool};

pub(crate) struct Fixture {
    pub directory: tempfile::TempDir,
    pub repo: SqliteCatalog,
    pub provider: Provider,
    pub post: SourcePost,
    pub collection: i64,
}
pub(crate) struct Provider {
    pub bytes: Vec<u8>,
    pub calls: Cell<u32>,
    pub posts: Vec<SourcePost>,
}
impl MediaProvider for Provider {
    fn posts(&self, _: &str, before: Option<i64>) -> Result<Vec<SourcePost>, IngestError> {
        Ok(self
            .posts
            .iter()
            .filter(|p| before.is_none_or(|id| p.id < id))
            .cloned()
            .collect())
    }
    fn download(&self, _: &SourcePost, path: &Path, _: &AtomicBool) -> Result<(), IngestError> {
        self.calls.set(self.calls.get() + 1);
        std::fs::write(path, &self.bytes).map_err(|_| IngestError::Storage)
    }
}
pub(crate) fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let mut repo = SqliteCatalog::open(&directory.path().join("catalog.sqlite3")).unwrap();
    let collection = repo
        .add(
            CollectionKind::Character,
            &SourceTag {
                id: 1,
                name: "character".into(),
                category: 4,
                post_count: 2,
            },
        )
        .unwrap()
        .record
        .id;
    let image = image::RgbImage::from_fn(800, 800, |x, y| {
        image::Rgb([(x % 255) as u8, (y % 255) as u8, ((x * y) % 255) as u8])
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let bytes = bytes.into_inner();
    let post = SourcePost {
        id: 100,
        md5: format!("{:x}", Md5::digest(&bytes)),
        file_url: Some("https://cdn.donmai.us/original/example.png".into()),
        file_ext: "png".into(),
        file_size: bytes.len() as u64,
        image_width: 800,
        image_height: 800,
        tag_string: "artist character scenery special_term".into(),
        tag_string_artist: "artist".into(),
        tag_string_character: "character".into(),
        tag_string_general: "scenery".into(),
        tag_string_copyright: String::new(),
        tag_string_meta: String::new(),
        rating: "g".into(),
        is_deleted: false,
    };
    let provider = Provider {
        bytes,
        calls: Cell::new(0),
        posts: vec![post.clone()],
    };
    Fixture {
        directory,
        repo,
        provider,
        post,
        collection,
    }
}

#[test]
fn transparent_import_is_flattened_to_white_and_tracks_both_hashes() {
    let mut f = fixture();
    let source_image = image::RgbaImage::from_fn(800, 800, |x, _| {
        if x < 400 {
            image::Rgba([220, 30, 30, 0])
        } else {
            image::Rgba([30, 60, 220, 255])
        }
    });
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(source_image)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let source_bytes = encoded.into_inner();
    f.provider.bytes = source_bytes.clone();
    f.post.md5 = format!("{:x}", Md5::digest(&source_bytes));
    f.post.file_size = source_bytes.len() as u64;
    let cancel = AtomicBool::new(false);
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            f.collection,
            800,
            &cancel,
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    let stored_bytes = std::fs::read(&media.path).unwrap();
    let stored = image::load_from_memory(&stored_bytes).unwrap().to_rgba8();
    assert_eq!(stored.get_pixel(100, 100).0, [255, 255, 255, 255]);
    assert_eq!(stored.get_pixel(700, 100).0, [30, 60, 220, 255]);
    assert_eq!(
        media.source_sha256,
        format!("{:x}", <Sha256 as Sha2Digest>::digest(&source_bytes))
    );
    assert_eq!(
        media.sha256,
        format!("{:x}", <Sha256 as Sha2Digest>::digest(&stored_bytes))
    );
    assert_ne!(media.source_sha256, media.sha256);

    let mut keep = fixture();
    keep.provider.bytes = source_bytes.clone();
    keep.post.md5 = format!("{:x}", Md5::digest(&source_bytes));
    keep.post.file_size = source_bytes.len() as u64;
    keep.repo
        .import_with_options(
            &keep.post,
            &keep.directory.path().join("library"),
            keep.collection,
            800,
            false,
            &cancel,
            &keep.provider,
        )
        .unwrap();
    let preserved = keep.repo.list_media().unwrap().remove(0);
    let preserved_image = image::load_from_memory(&std::fs::read(&preserved.path).unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(preserved_image.get_pixel(100, 100).0, [220, 30, 30, 0]);
    assert!(!preserved.flatten_transparency);
}

#[test]
fn network_download_overlaps_dual_signal_dedup_and_preserves_source_reuse() {
    let mut f = fixture();
    let root = f.directory.path().join("library");
    let cancel = AtomicBool::new(false);
    f.repo
        .import(&f.post, &root, f.collection, 800, &cancel, &f.provider)
        .unwrap();
    let first = f.repo.list_media().unwrap().remove(0);
    let mut second = f.post.clone();
    second.id = 99;
    let decoded = image::load_from_memory(&f.provider.bytes)
        .unwrap()
        .to_rgb8();
    let mut output = Vec::new();
    use image::ImageEncoder;
    image::codecs::png::PngEncoder::new_with_quality(
        &mut output,
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::NoFilter,
    )
    .write_image(decoded.as_raw(), 800, 800, image::ExtendedColorType::Rgb8)
    .unwrap();
    f.provider.bytes = output;
    second.md5 = format!("{:x}", Md5::digest(&f.provider.bytes));
    second.file_size = f.provider.bytes.len() as u64;
    f.repo
        .import(&second, &root, f.collection, 800, &cancel, &f.provider)
        .unwrap();
    let second_media = f
        .repo
        .list_media()
        .unwrap()
        .into_iter()
        .find(|m| m.id != first.id)
        .unwrap();
    // Compatible stored embeddings exercise real retrieval/deletion without starting Python.
    let mut vector = vec![0.0f32; 384];
    vector[0] = 1.0;
    for media in [&first, &second_media] {
        f.repo
            .connection
            .execute(
                "INSERT INTO visual_features VALUES(?1,?2,?3,?4)",
                rusqlite::params![
                    media.id,
                    crate::visual::VERSION,
                    format!("{:016x}", crate::workflows::phash(&media.path).unwrap()),
                    serde_json::to_string(&vector).unwrap()
                ],
            )
            .unwrap();
    }
    let mut third = second.clone();
    third.id = 98;
    let path = f.directory.path().join("catalog.sqlite3");
    let collection = f.collection;
    let (ready, started) = std::sync::mpsc::channel();
    let (release, proceed) = std::sync::mpsc::channel();
    let (done, completed) = std::sync::mpsc::channel();
    struct BlockedProvider {
        bytes: Vec<u8>,
        ready: std::sync::mpsc::Sender<()>,
        proceed: std::sync::mpsc::Receiver<()>,
    }
    impl MediaProvider for BlockedProvider {
        fn posts(&self, _: &str, _: Option<i64>) -> Result<Vec<SourcePost>, IngestError> {
            Ok(Vec::new())
        }
        fn download(&self, _: &SourcePost, path: &Path, _: &AtomicBool) -> Result<(), IngestError> {
            self.ready.send(()).unwrap();
            self.proceed
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            std::fs::write(path, &self.bytes).map_err(|_| IngestError::Storage)
        }
    }
    let import_root = root.clone();
    let provider = BlockedProvider {
        bytes: f.provider.bytes.clone(),
        ready,
        proceed,
    };
    let importer = std::thread::spawn(move || {
        let mut repo = SqliteCatalog::open(&path).unwrap();
        let result = repo.import(
            &third,
            &import_root,
            collection,
            800,
            &AtomicBool::new(false),
            &provider,
        );
        done.send(result).unwrap();
    });
    started
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let mut job = f
        .repo
        .new_workflow("dedup-auto", vec![first.id, second_media.id])
        .unwrap();
    crate::visual::run(
        &mut f.repo,
        &mut job,
        &root.join("missing-model"),
        &cancel,
        |_| {},
    )
    .unwrap();
    assert_eq!(job.state, "completed");
    assert_eq!(f.repo.list_media().unwrap().len(), 1);
    assert!(!second_media.path.exists());
    // Import is still waiting on network while the separate worker finished deletion.
    assert!(matches!(
        completed.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    release.send(()).unwrap();
    assert!(completed
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap());
    importer.join().unwrap();
    assert!(f.repo.has_source(&second, collection).unwrap());
    assert_eq!(f.repo.list_media().unwrap().len(), 2);
    assert!(f.repo.list_media().unwrap().iter().all(|m| m.path.is_file()
        && !std::fs::read_to_string(m.path.with_extension("tags.txt"))
            .unwrap()
            .is_empty()));
    assert!(f
        .repo
        .list_workflows()
        .unwrap()
        .iter()
        .all(|j| j.kind == "dedup-auto"));
    let violations: i64 = f
        .repo
        .connection
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
}

#[test]
fn character_import_discovers_all_artists_and_remote_lookup_reuses_their_ids() {
    let mut f = fixture();
    f.post.tag_string_artist = "artist collaborator artist".into();
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            f.collection,
            800,
            &AtomicBool::new(false),
            &f.provider,
        )
        .unwrap();
    let records = f.repo.list().unwrap();
    let artists: Vec<_> = records
        .iter()
        .filter(|record| record.kind == CollectionKind::Artist)
        .collect();
    assert_eq!(artists.len(), 2);
    assert!(artists.iter().all(|record| record.source.is_none()));
    let media = f.repo.list_media().unwrap().remove(0);
    assert_eq!(media.collections.len(), 3);
    assert!(artists
        .iter()
        .all(|record| media.collections.contains(&record.id)));
    let id = artists
        .iter()
        .find(|record| record.name == "artist")
        .unwrap()
        .id;
    let confirmed = f
        .repo
        .add(
            CollectionKind::Artist,
            &SourceTag {
                id: 22,
                name: "artist".into(),
                category: 1,
                post_count: 1234,
            },
        )
        .unwrap();
    assert!(!confirmed.created);
    assert_eq!(confirmed.record.id, id);
    assert_eq!(confirmed.record.source.unwrap().post_count, 1234);
    assert!(f.repo.has_source(&f.post, f.collection).unwrap());
    assert_eq!(f.provider.calls.get(), 1);
    assert_eq!(f.repo.list().unwrap().len(), 3);
    assert_eq!(
        f.repo.list_media().unwrap()[0].collections,
        media.collections
    );
}

#[test]
fn legacy_artist_backfill_reads_every_source_and_preserves_edits_tags_and_deletions() {
    let mut f = fixture();
    let root = f.directory.path().join("library");
    f.repo
        .import(
            &f.post,
            &root,
            f.collection,
            800,
            &AtomicBool::new(false),
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    let raw = std::fs::read(media.path.with_extension("raw_tags.txt")).unwrap();
    let tags = std::fs::read(media.path.with_extension("tags.txt")).unwrap();
    let mut additional = f.post.clone();
    additional.id = 99;
    additional.tag_string_artist = "second_artist".into();
    f.repo
        .connection
        .execute(
            "INSERT INTO media_sources(post_id,media_id,metadata) VALUES(?1,?2,?3)",
            rusqlite::params![
                additional.id,
                media.id,
                serde_json::to_string(&additional).unwrap()
            ],
        )
        .unwrap();
    // Simulate a library created before automatic artist discovery.
    f.repo
        .connection
        .execute("DELETE FROM collections WHERE category=1", [])
        .unwrap();
    let path = f.directory.path().join("catalog.sqlite3");
    drop(f.repo);
    let mut repo = SqliteCatalog::open(&path).unwrap();
    repo.attach_known_collections().unwrap();
    let records = repo.list().unwrap();
    assert!(records.iter().any(|record| record.name == "artist"));
    assert!(records.iter().any(|record| record.name == "second_artist"));
    let recovered = repo.list_media().unwrap().remove(0);
    assert_eq!(recovered.collections.len(), 3);
    assert_eq!(recovered.post, media.post);
    assert_eq!(recovered.sha256, media.sha256);
    assert_eq!(
        std::fs::read(media.path.with_extension("raw_tags.txt")).unwrap(),
        raw
    );
    assert_eq!(
        std::fs::read(media.path.with_extension("tags.txt")).unwrap(),
        tags
    );
    repo.attach_known_collections().unwrap();
    assert_eq!(repo.list().unwrap(), records);
    assert_eq!(repo.list_media().unwrap()[0], recovered);
    repo.delete_media(
        &std::collections::BTreeSet::from([media.id]),
        None,
        "test delete discovered artist scope",
    )
    .unwrap();
    let artist = records
        .iter()
        .find(|record| record.name == "artist")
        .unwrap()
        .id;
    repo.delete_empty(CollectionKind::Artist, &[artist])
        .unwrap();
    repo.attach_known_collections().unwrap();
    assert!(!repo
        .list()
        .unwrap()
        .iter()
        .any(|record| record.id == artist));
    assert!(repo.list_media().unwrap().is_empty());
}

#[test]
fn v3_discovery_migration_preserves_shared_media_jobs_and_collection_sequence() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.sqlite3");
    let connection = rusqlite::Connection::open(&path).unwrap();
    for (version, sql) in [
        (1, include_str!("migrations/001_collections.sql")),
        (2, include_str!("migrations/002_media.sql")),
        (3, include_str!("migrations/003_workflows.sql")),
    ] {
        connection.execute_batch(sql).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations(version,checksum) VALUES(?1,?2)",
                rusqlite::params![
                    version,
                    format!(
                        "{:x}",
                        sha2::Sha256::digest(sql.replace("\r\n", "\n").as_bytes())
                    )
                ],
            )
            .unwrap();
    }
    connection.execute("INSERT INTO collections(id,provider,source_tag_id,category,tag,remote_post_count) VALUES(7,'danbooru',71,4,'character',12)", []).unwrap();
    connection.execute("INSERT INTO collections(id,provider,source_tag_id,category,tag,remote_post_count) VALUES(20,'danbooru',72,1,'artist',10)", []).unwrap();
    connection
        .execute(
            "INSERT INTO media(id,md5,metadata) VALUES(9,'fixture','{}')",
            [],
        )
        .unwrap();
    connection
        .execute("INSERT INTO collection_media VALUES(7,9)", [])
        .unwrap();
    connection
        .execute(
            "INSERT INTO ingest_jobs(id,collection_id,metadata) VALUES(4,7,'{}')",
            [],
        )
        .unwrap();
    connection
        .execute("DELETE FROM collections WHERE id=20", [])
        .unwrap();
    drop(connection);
    let mut repo = SqliteCatalog::open(&path).unwrap();
    assert!(path.with_extension("pre-v3.sqlite3").is_file());
    assert_eq!(
        repo.connection
            .query_row(
                "SELECT count(*) FROM collection_media WHERE collection_id=7 AND media_id=9",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        repo.connection
            .query_row(
                "SELECT collection_id FROM ingest_jobs WHERE id=4",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        7
    );
    assert_eq!(
        repo.connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    let added = repo
        .add(
            CollectionKind::Artist,
            &SourceTag {
                id: 73,
                name: "another_artist".into(),
                category: 1,
                post_count: 0,
            },
        )
        .unwrap();
    assert!(added.record.id > 20);
    assert_eq!(
        repo.list().unwrap()[0].source.as_ref().unwrap().post_count,
        12
    );
}

#[test]
fn deletion_waits_for_local_tag_finalization_across_catalog_connections() {
    let mut f = fixture();
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            f.collection,
            800,
            &AtomicBool::new(false),
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    let path = f.directory.path().join("catalog.sqlite3");
    let other = SqliteCatalog::open(&path).unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &f.repo.media_lock,
        &other.media_lock
    ));
    let lock = f.repo.media_lock.clone();
    let guard = lock.lock().unwrap();
    let (ready, started) = std::sync::mpsc::channel();
    let (done, completed) = std::sync::mpsc::channel();
    let id = media.id;
    let deletion = std::thread::spawn(move || {
        let mut other = other;
        ready.send(()).unwrap();
        done.send(other.delete_media(
            &std::collections::BTreeSet::from([id]),
            None,
            "test coordinated deletion",
        ))
        .unwrap();
    });
    started.recv().unwrap();
    assert!(matches!(
        completed.recv_timeout(std::time::Duration::from_millis(100)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    f.repo
        .connection
        .execute("DELETE FROM text_artifacts WHERE media_id=?1", [id])
        .unwrap();
    f.repo.normalize_media_locked(&media).unwrap();
    assert!(
        !std::fs::read_to_string(media.path.with_extension("tags.txt"))
            .unwrap()
            .is_empty()
    );
    drop(guard);
    completed
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    deletion.join().unwrap();
    assert!(!media.path.exists());
    assert!(f.repo.list_media().unwrap().is_empty());
}

#[test]
fn analysis_batches_are_bounded_and_missing_model_items_wait_for_explicit_retry() {
    let mut f = fixture();
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            f.collection,
            800,
            &AtomicBool::new(false),
            &f.provider,
        )
        .unwrap();
    let id = f.repo.list_media().unwrap()[0].id;
    assert!(f.repo.pending_analysis_batch(0, true).unwrap().is_empty());
    assert_eq!(f.repo.pending_analysis_batch(1, false).unwrap(), vec![id]);
    let mut job = f.repo.new_workflow("dedup-auto", vec![id]).unwrap();
    crate::visual::run(
        &mut f.repo,
        &mut job,
        &f.directory.path().join("missing-model"),
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(job.state, "needs_model");
    assert!(f.repo.pending_analysis_batch(64, false).unwrap().is_empty());
    assert_eq!(f.repo.pending_analysis_batch(64, true).unwrap(), vec![id]);
    let mut reopened = SqliteCatalog::open(&f.directory.path().join("catalog.sqlite3")).unwrap();
    reopened.recover_workflows().unwrap();
    assert!(reopened
        .pending_analysis_batch(64, false)
        .unwrap()
        .is_empty());
    assert_eq!(reopened.pending_analysis_batch(64, true).unwrap(), vec![id]);
    reopened
        .connection
        .execute(
            "UPDATE analysis_jobs SET state='failed' WHERE media_id=?1",
            [id],
        )
        .unwrap();
    assert!(reopened
        .pending_analysis_batch(64, true)
        .unwrap()
        .is_empty());
}

#[test]
fn tag_write_waits_for_background_writer_instead_of_failing_read_to_write_upgrade() {
    let mut f = fixture();
    let root = f.directory.path().join("library");
    f.repo
        .import(
            &f.post,
            &root,
            f.collection,
            800,
            &AtomicBool::new(false),
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    let path = f.directory.path().join("catalog.sqlite3");
    let (ready, receiver) = std::sync::mpsc::channel();
    let background = std::thread::spawn(move || {
        let mut connection = rusqlite::Connection::open(path).unwrap();
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        tx.execute("UPDATE media SET keep=1 WHERE id=?1", [media.id])
            .unwrap();
        ready.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(150));
        tx.commit().unwrap();
    });
    receiver.recv().unwrap();
    // Previously the deferred transaction read tags while the writer held its reservation,
    // then failed immediately with SQLITE_BUSY on upgrade, ignoring busy_timeout.
    f.repo
        .write_artifact(
            &media,
            "tags",
            "recovered, character",
            "local-normalization-v1",
            "complete",
        )
        .unwrap();
    background.join().unwrap();
    assert_eq!(
        std::fs::read_to_string(media.path.with_extension("tags.txt")).unwrap(),
        "recovered, character"
    );
    assert!(f.repo.media_by_id(media.id).unwrap().unwrap().keep);
}

#[test]
fn committed_import_recovers_missing_tags_without_redownloading_or_new_versions() {
    let mut f = fixture();
    let root = f.directory.path().join("library");
    f.repo
        .import(
            &f.post,
            &root,
            f.collection,
            800,
            &AtomicBool::new(false),
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    // Reproduce the production state: media committed, local tag completion interrupted.
    f.repo
        .connection
        .execute("DELETE FROM text_artifacts WHERE media_id=?1", [media.id])
        .unwrap();
    std::fs::write(media.path.with_extension("tags.txt"), b"").unwrap();
    let mut reopened = SqliteCatalog::open(&f.directory.path().join("catalog.sqlite3")).unwrap();
    reopened.recover_import_tags().unwrap();
    assert!(reopened.has_source(&f.post, f.collection).unwrap());
    assert_eq!(f.provider.calls.get(), 1);
    assert!(
        !std::fs::read_to_string(media.path.with_extension("tags.txt"))
            .unwrap()
            .is_empty()
    );
    let versions: i64 = reopened
        .connection
        .query_row(
            "SELECT count(*) FROM text_artifacts WHERE media_id=?1",
            [media.id],
            |r| r.get(0),
        )
        .unwrap();
    reopened.recover_import_tags().unwrap();
    assert_eq!(
        reopened
            .connection
            .query_row(
                "SELECT count(*) FROM text_artifacts WHERE media_id=?1",
                [media.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        versions
    );
}

#[test]
fn database_busy_and_file_errors_have_distinct_redacted_diagnostics() {
    let f = fixture();
    let path = f.directory.path().join("catalog.sqlite3");
    let blocker = rusqlite::Connection::open(path).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    f.repo
        .connection
        .busy_timeout(std::time::Duration::ZERO)
        .unwrap();
    let error = crate::ingest::storage(f.repo.connection.execute("UPDATE media SET keep=0", []));
    assert_eq!(error, Err(IngestError::DatabaseBusy));
    blocker.execute_batch("ROLLBACK").unwrap();
    let secret_path = "credential-key-must-not-appear";
    let error = crate::ingest::storage::<(), _>(Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        secret_path,
    )))
    .unwrap_err();
    assert!(matches!(error, IngestError::StorageDetails(_)));
    assert!(!error.to_string().contains(secret_path));
}

#[test]
fn configured_short_side_filters_before_download_and_accepts_lowered_minimum() {
    for (width, height, minimum, imported) in [
        (800, 900, 800, true),
        (799, 900, 800, false),
        (900, 799, 800, false),
        (800, 900, 801, false),
        (640, 900, 640, true),
        (900, 640, 640, true),
    ] {
        let mut f = fixture();
        let image = image::RgbImage::new(width, height);
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        f.provider.bytes = bytes.into_inner();
        f.post.md5 = format!("{:x}", Md5::digest(&f.provider.bytes));
        f.post.file_size = f.provider.bytes.len() as u64;
        f.post.image_width = width;
        f.post.image_height = height;
        f.provider.posts = vec![f.post.clone()];
        let root = f.directory.path().join("library");
        let mut job = f
            .repo
            .new_job(f.collection, "character".into(), root, minimum)
            .unwrap();
        bdm_application::ingest::run(
            &f.provider,
            &mut f.repo,
            &mut job,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(
            job.progress.imported,
            u64::from(imported),
            "{width}x{height} at {minimum}"
        );
        assert_eq!(job.progress.skipped, u64::from(!imported));
        assert_eq!(f.provider.calls.get(), u32::from(imported));
        let mut reopened =
            SqliteCatalog::open(&f.directory.path().join("catalog.sqlite3")).unwrap();
        assert_eq!(reopened.list_jobs().unwrap()[0].minimum_short_side, minimum);
        if imported {
            let media = reopened.list_media().unwrap().remove(0);
            assert_eq!(media.minimum_short_side, minimum);
            // Recovery must retain the lower threshold, even after settings change/restart.
            reopened
                .connection
                .execute(
                    "INSERT INTO import_journal VALUES(?1,?2)",
                    rusqlite::params![media.post.md5, serde_json::to_string(&media).unwrap()],
                )
                .unwrap();
            reopened.recover_imports().unwrap();
            assert!(reopened
                .media_by_id(media.id)
                .unwrap()
                .unwrap()
                .path
                .is_file());
        }
    }
}

#[test]
fn configured_minimum_cannot_bypass_actual_dimensions_or_static_media_validation() {
    let mut f = fixture();
    let root = f.directory.path().join("library");
    let cancel = AtomicBool::new(false);
    // Real fixture is 800x800; a stricter threshold rejects before any byte request.
    assert_eq!(
        f.repo
            .import(&f.post, &root, f.collection, 900, &cancel, &f.provider),
        Err(IngestError::InvalidMedia)
    );
    assert_eq!(f.provider.calls.get(), 0);
    // Misreported original dimensions still fail post-download verification.
    f.post.image_width = 900;
    f.post.image_height = 900;
    assert_eq!(
        f.repo
            .import(&f.post, &root, f.collection, 900, &cancel, &f.provider),
        Err(IngestError::InvalidMedia)
    );
    assert_eq!(f.provider.calls.get(), 1);
    assert!(f.repo.list_media().unwrap().is_empty());
    f.post.tag_string.push_str(" animated");
    assert_eq!(
        f.repo
            .import(&f.post, &root, f.collection, 1, &cancel, &f.provider),
        Err(IngestError::InvalidMedia)
    );
    assert_eq!(f.provider.calls.get(), 1);
    for invalid in [0, 16385] {
        assert!(f
            .repo
            .new_job(f.collection, "character".into(), root.clone(), invalid)
            .is_err());
    }
    assert!(f.repo.list_jobs().unwrap().is_empty());
}

#[test]
fn legacy_job_and_promoted_media_default_to_800() {
    let mut f = fixture();
    let root = f.directory.path().join("library");
    let job = f
        .repo
        .new_job(f.collection, "character".into(), root.clone(), 1200)
        .unwrap();
    let mut json = serde_json::to_value(&job).unwrap();
    json.as_object_mut().unwrap().remove("minimum_short_side");
    let legacy: bdm_domain::ingest::IngestJob = serde_json::from_value(json).unwrap();
    assert_eq!(legacy.minimum_short_side, 800);
    f.repo
        .import(
            &f.post,
            &root,
            f.collection,
            800,
            &AtomicBool::new(false),
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    let mut json = serde_json::to_value(&media).unwrap();
    json.as_object_mut().unwrap().remove("minimum_short_side");
    f.repo
        .connection
        .execute(
            "INSERT INTO import_journal VALUES(?1,?2)",
            rusqlite::params![media.post.md5, serde_json::to_string(&json).unwrap()],
        )
        .unwrap();
    f.repo.recover_imports().unwrap();
    assert_eq!(f.repo.list_media().unwrap()[0].minimum_short_side, 800);
}

#[test]
fn import_reopen_dedup_sources_sidecars_and_review_preservation() {
    let mut f = fixture();
    let root = f.directory.path().join("library");
    let cancel = AtomicBool::new(false);
    assert!(f
        .repo
        .import(&f.post, &root, f.collection, 800, &cancel, &f.provider)
        .unwrap());
    assert!(!f
        .repo
        .import(&f.post, &root, f.collection, 800, &cancel, &f.provider)
        .unwrap());
    assert_eq!(f.provider.calls.get(), 1);
    let first = f.repo.list_media().unwrap().remove(0);
    assert!(first.path.is_file() && first.thumbnail.is_file());
    assert_eq!(
        std::fs::read_to_string(first.path.with_extension("raw_tags.txt")).unwrap(),
        f.post.tag_string
    );
    let tags = f.repo.list_artifacts().unwrap();
    assert!(tags[0].content.contains("@artist artist"));
    assert!(tags[0].content.contains("special_term"));
    let mut second = f.post.clone();
    second.id = 101;
    assert!(!f
        .repo
        .import(&second, &root, f.collection, 800, &cancel, &f.provider)
        .unwrap());
    assert_eq!(f.provider.calls.get(), 2);
    assert_eq!(f.repo.list_media().unwrap().len(), 1);
    let count: i64 = f
        .repo
        .connection
        .query_row("SELECT count(*) FROM media_sources", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
    f.repo
        .write_artifact(
            &first,
            "caption",
            "A landscape.",
            "test-api",
            "needs_review",
        )
        .unwrap();
    f.repo.review_artifact(first.id, "caption").unwrap();
    f.repo
        .write_artifact(
            &first,
            "caption",
            "Overwrite forbidden.",
            "test-api",
            "complete",
        )
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(first.path.with_extension("txt")).unwrap(),
        "A landscape."
    );
    f.repo.set_media_keep(first.id, true).unwrap();
    let reopened = SqliteCatalog::open(&f.directory.path().join("catalog.sqlite3")).unwrap();
    assert!(reopened.list_media().unwrap()[0].keep);
    assert!(f
        .repo
        .delete_empty(CollectionKind::Character, &[f.collection])
        .is_err());
}

#[test]
fn metadata_refresh_preserves_optimized_tags_and_human_relation_removals() {
    let mut f = fixture();
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            f.collection,
            800,
            &AtomicBool::new(false),
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    f.repo
        .write_artifact(
            &media,
            "tags",
            "scenery, @artist artist, character, special_term",
            "ai:test",
            "needs_review",
        )
        .unwrap();
    let mut rules = f.repo.tag_rules().unwrap();
    rules
        .rejected_characters
        .insert(("base".into(), "character".into()));
    rules.resolved_aliases.insert("scenery".into());
    f.repo.save_tag_rules(&rules).unwrap();
    f.repo.normalize_media(&media).unwrap();
    let artifact = f.repo.list_artifacts().unwrap().remove(0);
    assert_eq!(artifact.provenance, "ai:test");
    assert_eq!(
        artifact.content,
        "scenery, @artist artist, character, special_term"
    );
    assert_eq!(
        std::fs::read_to_string(media.path.with_extension("tags.txt")).unwrap(),
        artifact.content
    );
    let reopened = SqliteCatalog::open(&f.directory.path().join("catalog.sqlite3")).unwrap();
    assert!(reopened
        .tag_rules()
        .unwrap()
        .rejected_characters
        .contains(&("base".into(), "character".into())));
    assert!(reopened
        .tag_rules()
        .unwrap()
        .resolved_aliases
        .contains("scenery"));
}

#[test]
fn saved_exclusivity_groups_preserve_coexisting_variants_and_complete_local_tags() {
    let mut f = fixture();
    f.post.tag_string = "artist base character other_variant unrelated scenery".into();
    f.post.tag_string_character = "base character other_variant unrelated".into();
    let mut rules = f.repo.tag_rules().unwrap();
    rules.categories.insert("artist".into(), 1);
    rules.categories.insert("scenery".into(), 0);
    for name in ["base", "character", "other_variant", "unrelated"] {
        rules.categories.insert(name.into(), 4);
    }
    for variant in ["character", "other_variant"] {
        rules.characters.push(bdm_domain::tags::CharacterRelation {
            base: "base".into(),
            variant: variant.into(),
            verified: true,
            exclusive_group: "costume".into(),
            evidence: "human-confirmed".into(),
        });
    }
    f.repo.save_tag_rules(&rules).unwrap();
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            f.collection,
            800,
            &AtomicBool::new(false),
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    let expected = "@artist artist, character, other_variant, unrelated, scenery";
    // Reopen with a persisted legacy group: existing rules must not flag co-occurrence.
    let mut reopened = SqliteCatalog::open(&f.directory.path().join("catalog.sqlite3")).unwrap();
    let result = reopened.normalize_media(&media).unwrap();
    assert_eq!(
        result.characters,
        vec!["character", "other_variant", "unrelated"]
    );
    assert!(result.warnings.is_empty());
    let artifact = reopened.list_artifacts().unwrap().remove(0);
    assert_eq!(artifact.content, expected);
    assert_eq!(artifact.state, "complete");
    assert_eq!(artifact.provenance, "local-normalization-v1");
    assert_eq!(
        std::fs::read_to_string(media.path.with_extension("tags.txt")).unwrap(),
        expected
    );
    assert_eq!(
        std::fs::read_to_string(media.path.with_extension("raw_tags.txt")).unwrap(),
        f.post.tag_string
    );
    assert_eq!(reopened.tag_rules().unwrap(), rules);
}

#[test]
fn automatic_source_relation_updates_local_tags_but_preserves_raw_caption_and_reviewed_text() {
    let mut f = fixture();
    f.post.tag_string.push_str(" base");
    f.post.tag_string_character.push_str(" base");
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            f.collection,
            800,
            &AtomicBool::new(false),
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    let raw_path = media.path.with_extension("raw_tags.txt");
    let raw = std::fs::read(&raw_path).unwrap();
    f.repo
        .write_artifact(&media, "caption", "A scene.", "ai:test", "needs_review")
        .unwrap();
    let mut rules = f.repo.tag_rules().unwrap();
    rules
        .categories
        .extend([("base".into(), 4), ("character".into(), 4)]);
    assert!(rules.accept_character_implication("base", "character"));
    f.repo.save_tag_rules(&rules).unwrap();
    f.repo.normalize_media(&media).unwrap();
    let tags = std::fs::read_to_string(media.path.with_extension("tags.txt")).unwrap();
    assert!(!tags.split(", ").any(|t| t == "base"));
    assert!(tags.split(", ").any(|t| t == "character"));
    assert_eq!(std::fs::read(&raw_path).unwrap(), raw);
    assert_eq!(
        std::fs::read_to_string(media.path.with_extension("txt")).unwrap(),
        "A scene."
    );
    f.repo.review_artifact(media.id, "tags").unwrap();
    rules.characters.clear();
    f.repo.save_tag_rules(&rules).unwrap();
    f.repo.normalize_media(&media).unwrap();
    assert_eq!(
        std::fs::read_to_string(media.path.with_extension("tags.txt")).unwrap(),
        tags
    );
}

#[test]
fn failed_download_queue_advances_and_resume_only_retries_failed_items() {
    struct FailingProvider {
        inner: Provider,
        failed: Cell<bool>,
        seen: std::cell::RefCell<Vec<i64>>,
    }
    impl MediaProvider for FailingProvider {
        fn posts(&self, tag: &str, before: Option<i64>) -> Result<Vec<SourcePost>, IngestError> {
            self.inner.posts(tag, before)
        }
        fn download(
            &self,
            post: &SourcePost,
            path: &Path,
            cancel: &AtomicBool,
        ) -> Result<(), IngestError> {
            self.seen.borrow_mut().push(post.id);
            if post.id == 100 && self.failed.get() {
                return Err(IngestError::RateLimited(0));
            }
            self.inner.download(post, path, cancel)
        }
    }
    let mut f = fixture();
    let mut second = f.post.clone();
    second.id = 99;
    f.provider.posts.push(second);
    let provider = FailingProvider {
        inner: f.provider,
        failed: Cell::new(true),
        seen: Default::default(),
    };
    let mut job = f
        .repo
        .new_job(
            f.collection,
            "character".into(),
            f.directory.path().join("library"),
            640,
        )
        .unwrap();
    bdm_application::ingest::run(
        &provider,
        &mut f.repo,
        &mut job,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(job.state, "deferred");
    assert_eq!(job.before_id, Some(99));
    assert_eq!(job.progress.examined, 2);
    assert_eq!(job.progress.imported, 1);
    assert_eq!(job.progress.failed, 1);
    assert_eq!(*provider.seen.borrow(), vec![100, 100, 100, 99]);
    let mut reopened = SqliteCatalog::open(&f.directory.path().join("catalog.sqlite3")).unwrap();
    let mut job = reopened.list_jobs().unwrap().remove(0);
    assert_eq!(job.minimum_short_side, 640);
    assert_eq!(job.failed_posts.len(), 1);
    provider.failed.set(false);
    bdm_application::ingest::run(
        &provider,
        &mut reopened,
        &mut job,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(job.state, "completed");
    assert!(job.failed_posts.is_empty());
    assert_eq!(job.progress.failed, 0);
    assert_eq!(job.progress.examined, 2);
    assert_eq!(*provider.seen.borrow(), vec![100, 100, 100, 99, 100]);
    assert_eq!(reopened.list_media().unwrap().len(), 1);
}

#[test]
fn analysis_failures_pause_and_resume_preserve_attempt_budget_and_originals() {
    let mut f = fixture();
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            f.collection,
            800,
            &AtomicBool::new(false),
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    // A missing original exercises per-item errors without starting Python or the network.
    let held = media.path.with_extension("held");
    std::fs::rename(&media.path, &held).unwrap();
    let mut job = f.repo.new_workflow("dedup", vec![media.id]).unwrap();
    crate::visual::run(
        &mut f.repo,
        &mut job,
        f.directory.path(),
        &AtomicBool::new(true),
        |_| {},
    )
    .unwrap();
    assert_eq!(job.state, "paused");
    assert!(job.analysis_attempts.is_empty());
    job.analysis_attempts.insert(media.id, 2);
    f.repo.save_workflow(&job).unwrap();
    let mut repo = SqliteCatalog::open(&f.directory.path().join("catalog.sqlite3")).unwrap();
    let mut job = repo.list_workflows().unwrap().remove(0);
    crate::visual::run(
        &mut repo,
        &mut job,
        f.directory.path(),
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(job.state, "deferred");
    assert_eq!(job.analysis_attempts[&media.id], 3);
    assert_eq!(job.analysis_errors[&media.id].len(), 1);
    assert_eq!(job.deferred, vec![media.id]);
    assert!(held.is_file());
    assert!(repo.media_by_id(media.id).unwrap().is_some());
    std::fs::rename(held, &media.path).unwrap();
}

#[test]
fn corrupt_or_cancelled_import_never_commits_a_partial_original() {
    let mut f = fixture();
    let root = f.directory.path().join("library");
    f.provider.bytes[30] ^= 1;
    assert!(matches!(
        f.repo.import(
            &f.post,
            &root,
            f.collection,
            800,
            &AtomicBool::new(false),
            &f.provider
        ),
        Err(IngestError::Integrity)
    ));
    assert!(f.repo.list_media().unwrap().is_empty());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    let f = fixture();
    let mut repo = SqliteCatalog::in_memory().unwrap();
    assert!(matches!(
        repo.import(&f.post, &root, 1, 800, &AtomicBool::new(true), &f.provider),
        Err(IngestError::Cancelled)
    ));
    assert!(repo.list_media().unwrap().is_empty());
}

#[test]
fn dedup_delete_consolidates_sources_and_associations_permanently() {
    let mut f = fixture();
    let root = f.directory.path().join("library");
    let cancel = AtomicBool::new(false);
    f.repo
        .import(&f.post, &root, f.collection, 800, &cancel, &f.provider)
        .unwrap();
    let first = f.repo.list_media().unwrap().remove(0);
    let another = f
        .repo
        .add(
            CollectionKind::Artist,
            &SourceTag {
                id: 2,
                name: "other_artist".into(),
                category: 1,
                post_count: 1,
            },
        )
        .unwrap()
        .record
        .id;
    let mut second = f.post.clone();
    second.id = 99;
    // Same visual pixels with different PNG compression creates different bytes.
    let decoded = image::load_from_memory(&f.provider.bytes)
        .unwrap()
        .to_rgb8();
    let mut output = Vec::new();
    use image::ImageEncoder;
    image::codecs::png::PngEncoder::new_with_quality(
        &mut output,
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::NoFilter,
    )
    .write_image(decoded.as_raw(), 800, 800, image::ExtendedColorType::Rgb8)
    .unwrap();
    f.provider.bytes = output;
    second.md5 = format!("{:x}", Md5::digest(&f.provider.bytes));
    second.file_size = f.provider.bytes.len() as u64;
    f.repo
        .import(&second, &root, another, 800, &cancel, &f.provider)
        .unwrap();
    let second = f
        .repo
        .list_media()
        .unwrap()
        .into_iter()
        .find(|m| m.id != first.id)
        .unwrap();
    assert_eq!(
        crate::workflows::phash(&first.path).unwrap(),
        crate::workflows::phash(&second.path).unwrap()
    );
    f.repo
        .delete_media(
            &std::collections::BTreeSet::from([second.id]),
            Some(first.id),
            "test dual signals",
        )
        .unwrap();
    assert!(!second.path.exists());
    let all = f.repo.list_media().unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].collections.contains(&another));
    assert!(f.repo.has_source(&second.post, another).unwrap());
    let audits: i64 = f
        .repo
        .connection
        .query_row("SELECT count(*) FROM audit_log", [], |r| r.get(0))
        .unwrap();
    assert!(audits > 0);
    f.repo.recover_deletions().unwrap();
    assert_eq!(f.repo.list_media().unwrap().len(), 1);
}

#[test]
fn visual_review_decisions_preserve_survivor_sources_and_reject_stale_pairs() {
    for keep_first in [true, false] {
        let mut f = fixture();
        let root = f.directory.path().join("library");
        let cancel = AtomicBool::new(false);
        f.repo
            .import(&f.post, &root, f.collection, 800, &cancel, &f.provider)
            .unwrap();
        let first = f.repo.list_media().unwrap().remove(0);
        let mut post = f.post.clone();
        post.id = 99;
        post.tag_string_artist = "second_artist".into();
        let mut decoded = image::load_from_memory(&f.provider.bytes)
            .unwrap()
            .to_rgb8();
        decoded.put_pixel(0, 0, image::Rgb([255, 255, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(decoded)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        f.provider.bytes = bytes.into_inner();
        post.md5 = format!("{:x}", Md5::digest(&f.provider.bytes));
        post.file_size = f.provider.bytes.len() as u64;
        f.repo
            .import(&post, &root, f.collection, 800, &cancel, &f.provider)
            .unwrap();
        let second = f
            .repo
            .list_media()
            .unwrap()
            .into_iter()
            .find(|media| media.id != first.id)
            .unwrap();
        let pair = bdm_domain::dedup::DuplicateCandidate {
            first: first.id,
            second: second.id,
            phash_distance: Some(12),
            cosine: Some(0.96),
            model_version: Some("fixture".into()),
            state: "pending".into(),
        };
        f.repo.save_candidate(&pair).unwrap();
        let hashes = [first.sha256.clone(), second.sha256.clone()];
        assert_eq!(
            f.repo.resolve_candidate(
                &pair,
                &["stale".into(), second.sha256.clone()],
                Some(first.id)
            ),
            Err(IngestError::Integrity)
        );
        let mut stale = pair.clone();
        stale.cosine = Some(0.97);
        assert_eq!(
            f.repo.resolve_candidate(&stale, &hashes, Some(first.id)),
            Err(IngestError::Integrity)
        );
        assert!(first.path.is_file() && second.path.is_file());
        // Rejecting the relation keeps both originals and survives reopening/re-analysis.
        f.repo.resolve_candidate(&pair, &hashes, None).unwrap();
        assert_eq!(f.repo.list_media().unwrap().len(), 2);
        assert!(f.repo.candidate_rejected(first.id, second.id).unwrap());
        f.repo.save_candidate(&pair).unwrap();
        assert_eq!(f.repo.list_candidates().unwrap()[0].state, "rejected");
        assert_eq!(
            f.repo.resolve_candidate(&pair, &hashes, Some(first.id)),
            Err(IngestError::Integrity)
        );
        // A fresh pending fixture exercises each explicit deletion direction.
        f.repo
            .connection
            .execute("DELETE FROM duplicate_candidates", [])
            .unwrap();
        f.repo.save_candidate(&pair).unwrap();
        let (keep, remove) = if keep_first {
            (&first, &second)
        } else {
            (&second, &first)
        };
        let keep_bytes = std::fs::read(&keep.path).unwrap();
        let original_tags = std::fs::read(keep.path.with_extension("raw_tags.txt")).unwrap();
        f.repo
            .resolve_candidate(&pair, &hashes, Some(keep.id))
            .unwrap();
        assert!(!remove.path.exists());
        assert_eq!(std::fs::read(&keep.path).unwrap(), keep_bytes);
        assert_eq!(
            std::fs::read(keep.path.with_extension("raw_tags.txt")).unwrap(),
            original_tags
        );
        let media = f.repo.list_media().unwrap();
        assert_eq!(media.len(), 1);
        assert!(first
            .collections
            .union(&second.collections)
            .all(|id| media[0].collections.contains(id)));
        assert!(f.repo.has_source(&f.post, f.collection).unwrap());
        assert!(f.repo.has_source(&post, f.collection).unwrap());
        assert!(f.repo.list_candidates().unwrap().is_empty());
        assert_eq!(
            f.repo
                .connection
                .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
    }
}

#[test]
fn promoted_import_recovers_and_missing_sidecars_materialize() {
    let mut f = fixture();
    let root = f.directory.path().join("library");
    f.repo
        .import(
            &f.post,
            &root,
            f.collection,
            800,
            &AtomicBool::new(false),
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    f.repo
        .connection
        .execute(
            "INSERT INTO import_journal VALUES(?1,?2)",
            rusqlite::params![media.post.md5, serde_json::to_string(&media).unwrap()],
        )
        .unwrap();
    f.repo
        .connection
        .execute("DELETE FROM media WHERE id=?1", [media.id])
        .unwrap();
    f.repo.recover_imports().unwrap();
    assert_eq!(f.repo.list_media().unwrap().len(), 1);
    let media = f.repo.list_media().unwrap().remove(0);
    f.repo
        .write_artifact(&media, "caption", "Recovered content.", "test", "complete")
        .unwrap();
    std::fs::remove_file(media.path.with_extension("txt")).unwrap();
    f.repo.materialize_artifacts().unwrap();
    assert_eq!(
        std::fs::read_to_string(media.path.with_extension("txt")).unwrap(),
        "Recovered content."
    );
}

#[test]
fn legacy_v1_catalog_migrates_with_records_intact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("v1.sqlite3");
    let connection = rusqlite::Connection::open(&path).unwrap();
    let migration = include_str!("migrations/001_collections.sql");
    connection.execute_batch(migration).unwrap();
    use sha2::Sha256;
    connection
        .execute(
            "INSERT INTO schema_migrations(version,checksum) VALUES(1,?1)",
            [format!(
                "{:x}",
                Sha256::digest(migration.replace("\r\n", "\n").as_bytes())
            )],
        )
        .unwrap();
    connection.execute("INSERT INTO collections(provider,source_tag_id,category,tag,remote_post_count) VALUES('danbooru',1,4,'character',42)",[]).unwrap();
    drop(connection);
    let repo = SqliteCatalog::open(&path).unwrap();
    assert_eq!(
        repo.list().unwrap()[0].source.as_ref().unwrap().post_count,
        42
    );
    assert!(repo.list_media().unwrap().is_empty());
    let version: i64 = repo
        .connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 4);
}

#[test]
fn ai_local_http_round_trip_protects_names_and_isolates_caption() {
    let mut f = fixture();
    let root = f.directory.path().join("library");
    let cancel = AtomicBool::new(false);
    f.repo
        .import(&f.post, &root, f.collection, 800, &cancel, &f.provider)
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let media_id = media.id;
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};
        for mode in ["tags", "caption"] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(offset) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..offset]);
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= offset + 4 + length {
                        let body: serde_json::Value =
                            serde_json::from_slice(&bytes[offset + 4..offset + 4 + length])
                                .unwrap();
                        let content = body["messages"][1]["content"].as_str().unwrap();
                        assert!(
                            !content.contains("artist")
                                && !content.contains("character")
                                && !content.contains("special_term")
                        );
                        let result = if mode == "tags" {
                            serde_json::json!({"id":media_id,"tags":["scenery"]})
                        } else {
                            serde_json::json!({"id":media_id,"caption":"A landscape scene."})
                        };
                        let response=serde_json::json!({"choices":[{"message":{"content":serde_json::json!({"schema":"bdm-text-v1","items":[result]}).to_string()}}]}).to_string();
                        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
                        break;
                    }
                }
            }
        }
    });
    let profile = crate::ai::AiProfile {
        id: "local-test".into(),
        endpoint: format!("http://{address}/v1"),
        model: "test".into(),
        group: String::new(),
        credentials: None,
        settings: Default::default(),
        upload_approval: None,
        artist_tag_format: "legacy".into(),
        character_tag_format: "plain".into(),
    };
    let raw = std::fs::read(media.path.with_extension("raw_tags.txt")).unwrap();
    let mut tags = f.repo.new_workflow("optimize", vec![media.id]).unwrap();
    tags.mode = "tags".into();
    crate::ai::run(&mut f.repo, &mut tags, &profile, &cancel, |_| {}).unwrap();
    assert_eq!(tags.accepted.len(), 1);
    let tag_content = std::fs::read(media.path.with_extension("tags.txt")).unwrap();
    assert!(String::from_utf8_lossy(&tag_content).contains("@artist artist"));
    assert!(String::from_utf8_lossy(&tag_content).contains("character"));
    let mut caption = f.repo.new_workflow("optimize", vec![media.id]).unwrap();
    caption.mode = "caption".into();
    crate::ai::run(&mut f.repo, &mut caption, &profile, &cancel, |_| {}).unwrap();
    server.join().unwrap();
    assert_eq!(
        std::fs::read(media.path.with_extension("tags.txt")).unwrap(),
        tag_content
    );
    assert_eq!(
        std::fs::read(media.path.with_extension("raw_tags.txt")).unwrap(),
        raw
    );
    assert!(f
        .repo
        .list_artifacts()
        .unwrap()
        .iter()
        .any(|a| a.kind == "caption" && a.state == "needs_review"));
}

#[test]
fn ai_partial_batch_recovery_only_dispatches_failed_ids_and_resume_is_idempotent() {
    use std::io::{Read, Write};
    let mut f = fixture();
    let root = f.directory.path().join("library");
    let cancel = AtomicBool::new(false);
    f.repo
        .import(&f.post, &root, f.collection, 800, &cancel, &f.provider)
        .unwrap();
    let first = f.repo.list_media().unwrap().remove(0);
    let mut second = f.post.clone();
    second.id = 99;
    let mut decoded = image::load_from_memory(&f.provider.bytes)
        .unwrap()
        .to_rgb8();
    decoded.put_pixel(0, 0, image::Rgb([255, 255, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(decoded)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    f.provider.bytes = bytes.into_inner();
    second.md5 = format!("{:x}", Md5::digest(&f.provider.bytes));
    second.file_size = f.provider.bytes.len() as u64;
    f.repo
        .import(&second, &root, f.collection, 800, &cancel, &f.provider)
        .unwrap();
    let second = f
        .repo
        .list_media()
        .unwrap()
        .into_iter()
        .find(|m| m.id != first.id)
        .unwrap();
    let first_id = first.id;
    let second_id = second.id;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for attempt in 0..2 {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "missing recovery request"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            let input: serde_json::Value = loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(offset) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..offset]);
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= offset + 4 + length {
                        let body: serde_json::Value =
                            serde_json::from_slice(&bytes[offset + 4..offset + 4 + length])
                                .unwrap();
                        break serde_json::from_str(
                            body["messages"][1]["content"].as_str().unwrap(),
                        )
                        .unwrap();
                    }
                }
            };
            seen.push(
                input["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| row["id"].as_i64().unwrap())
                    .collect::<Vec<_>>(),
            );
            let id = if attempt == 0 { first_id } else { second_id };
            let response=serde_json::json!({"choices":[{"message":{"content":serde_json::json!({"schema":"bdm-text-v1","items":[{"id":id,"tags":["scenery"]}]}).to_string()}}]}).to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
        }
        seen
    });
    let profile = crate::ai::AiProfile {
        id: "partial-test".into(),
        endpoint: format!("http://{address}/v1"),
        model: "test".into(),
        group: String::new(),
        credentials: None,
        settings: Default::default(),
        upload_approval: None,
        artist_tag_format: "legacy".into(),
        character_tag_format: "plain".into(),
    };
    let mut job = f
        .repo
        .new_workflow("optimize", vec![first.id, second.id])
        .unwrap();
    job.mode = "tags".into();
    crate::ai::run(&mut f.repo, &mut job, &profile, &cancel, |_| {}).unwrap();
    assert_eq!(
        server.join().unwrap(),
        vec![vec![first.id, second.id], vec![second.id]]
    );
    assert_eq!(job.state, "completed");
    assert_eq!(job.accepted.len(), 2);
    assert!(job.deferred.is_empty());
    let count: i64 = f
        .repo
        .connection
        .query_row("SELECT count(*) FROM text_artifacts", [], |r| r.get(0))
        .unwrap();
    let mut reopened = SqliteCatalog::open(&f.directory.path().join("catalog.sqlite3")).unwrap();
    let mut job = reopened.list_workflows().unwrap().remove(0);
    // The HTTP server has exited; a correct resume makes no network request or new artifact.
    crate::ai::run(&mut reopened, &mut job, &profile, &cancel, |_| {}).unwrap();
    assert_eq!(
        count,
        reopened
            .connection
            .query_row("SELECT count(*) FROM text_artifacts", [], |r| r
                .get::<_, i64>(0))
            .unwrap()
    );
}

#[test]
fn export_reconciles_hashes_and_preserves_selected_text_format() {
    let mut f = fixture();
    let root = f.directory.path().join("library");
    let cancel = AtomicBool::new(false);
    f.repo
        .import(&f.post, &root, f.collection, 800, &cancel, &f.provider)
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    f.repo.set_media_keep(media.id, true).unwrap();
    f.repo
        .write_artifact(&media, "caption", "A landscape.", "test", "complete")
        .unwrap();
    let mut job = f.repo.new_workflow("export", vec![media.id]).unwrap();
    job.mode = "caption".into();
    let destination = f.directory.path().join("导出 目录");
    crate::export::run(&mut f.repo, &mut job, &destination, &cancel, |_| {}).unwrap();
    let directory = destination.join(format!("media_{}", media.id));
    assert_eq!(
        std::fs::read_to_string(directory.join(format!("media_{}.txt", media.id))).unwrap(),
        "A landscape."
    );
    assert!(directory.join("manifest.json").is_file());
    job.cursor = 0;
    crate::export::run(&mut f.repo, &mut job, &destination, &cancel, |_| {}).unwrap();
    std::fs::write(
        directory.join(format!("media_{}.png", media.id)),
        b"corrupt",
    )
    .unwrap();
    job.cursor = 0;
    assert!(crate::export::run(&mut f.repo, &mut job, &destination, &cancel, |_| {}).is_err());
}

#[test]
fn flat_export_updates_only_owned_txt_and_rejects_modified_pairs() {
    let mut f = fixture();
    let cancel = AtomicBool::new(false);
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            f.collection,
            800,
            &cancel,
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    f.repo.set_media_keep(media.id, true).unwrap();
    let root = f.directory.path().join("flat dataset");
    let base = format!("media_{}", media.id);
    let img_path = root.join(format!("{base}.png"));
    let txt_path = root.join(format!("{base}.txt"));
    f.repo
        .write_artifact(&media, "caption", "First description.", "test", "complete")
        .unwrap();
    let mut job = f.repo.new_workflow("export", vec![media.id]).unwrap();
    job.mode = "caption".into();
    job.export_layout = "flat".into();
    crate::export::run(&mut f.repo, &mut job, &root, &cancel, |_| {}).unwrap();
    assert!(img_path.is_file());
    assert_eq!(
        std::fs::read_to_string(&txt_path).unwrap(),
        "First description."
    );
    let original = std::fs::read(&img_path).unwrap();
    f.repo
        .write_artifact(
            &media,
            "caption",
            "Better description.",
            "test:2",
            "complete",
        )
        .unwrap();
    job.cursor = 0;
    crate::export::run(&mut f.repo, &mut job, &root, &cancel, |_| {}).unwrap();
    assert_eq!(
        std::fs::read_to_string(&txt_path).unwrap(),
        "Better description."
    );
    assert_eq!(std::fs::read(&img_path).unwrap(), original);
    std::fs::write(&txt_path, "Manual edit").unwrap();
    job.cursor = 0;
    assert!(matches!(
        crate::export::run(&mut f.repo, &mut job, &root, &cancel, |_| {}),
        Err(IngestError::Integrity)
    ));
}

#[test]
fn export_filters_redundant_character_bases_without_rewriting_reviewed_artifacts() {
    let mut f = fixture();
    let cancel = AtomicBool::new(false);
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            f.collection,
            800,
            &cancel,
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    let raw_path = media.path.with_extension("raw_tags.txt");
    let original_raw = std::fs::read(&raw_path).unwrap();
    let mut rules = f.repo.tag_rules().unwrap();
    rules
        .categories
        .extend([("base".into(), 4), ("specific".into(), 4)]);
    assert!(rules.accept_character_implication("base", "specific"));
    f.repo.save_tag_rules(&rules).unwrap();
    f.repo.set_media_keep(media.id, true).unwrap();
    for (index, source, expected) in [
        (0, "base, solo", "base, solo"),
        (1, "base, specific, another", "specific, another"),
    ] {
        f.repo
            .write_artifact(&media, "tags", source, "ai:test", "complete")
            .unwrap();
        if index == 1 {
            f.repo.review_artifact(media.id, "tags").unwrap();
        }
        let mut job = f.repo.new_workflow("export", vec![media.id]).unwrap();
        job.mode = "tags".into();
        let destination = f.directory.path().join(format!("export-{index}"));
        crate::export::run(&mut f.repo, &mut job, &destination, &cancel, |_| {}).unwrap();
        let folder = destination.join(format!("media_{}", media.id));
        let actual =
            std::fs::read_to_string(folder.join(format!("media_{}.txt", media.id))).unwrap();
        assert_eq!(actual, expected);
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(folder.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(
            manifest["text_sha256"],
            format!("{:x}", Sha256::digest(actual.as_bytes()))
        );
        assert_eq!(
            f.repo
                .list_artifacts()
                .unwrap()
                .into_iter()
                .find(|a| a.kind == "tags")
                .unwrap()
                .content,
            source
        );
        job.cursor = 0;
        crate::export::run(&mut f.repo, &mut job, &destination, &cancel, |_| {}).unwrap();
    }
    assert_eq!(std::fs::read(raw_path).unwrap(), original_raw);
}

#[test]
#[ignore = "Runs pinned local DINOv3 model and performs real two-signal duplicate analysis on temporary synthetic images"]
fn local_dinov3_scan_preserves_single_signal_and_removes_dual_match() {
    let mut f = fixture();
    let root = f.directory.path().join("library");
    let cancel = AtomicBool::new(false);
    f.repo
        .import(&f.post, &root, f.collection, 800, &cancel, &f.provider)
        .unwrap();
    let mut second = f.post.clone();
    second.id = 99;
    let decoded = image::load_from_memory(&f.provider.bytes)
        .unwrap()
        .to_rgb8();
    let mut output = Vec::new();
    use image::ImageEncoder;
    image::codecs::png::PngEncoder::new_with_quality(
        &mut output,
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::NoFilter,
    )
    .write_image(decoded.as_raw(), 800, 800, image::ExtendedColorType::Rgb8)
    .unwrap();
    f.provider.bytes = output;
    second.md5 = format!("{:x}", Md5::digest(&f.provider.bytes));
    second.file_size = f.provider.bytes.len() as u64;
    f.repo
        .import(&second, &root, f.collection, 800, &cancel, &f.provider)
        .unwrap();
    let ids = f.repo.list_media().unwrap().iter().map(|m| m.id).collect();
    let mut job = f.repo.new_workflow("dedup", ids).unwrap();
    crate::visual::run(
        &mut f.repo,
        &mut job,
        &root.join("missing-model"),
        &cancel,
        |_| {},
    )
    .unwrap();
    assert_eq!(f.repo.list_media().unwrap().len(), 2);
    assert_eq!(f.repo.list_candidates().unwrap().len(), 1);
    assert_eq!(job.state, "needs_model");
    job.cursor = 0;
    let model = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/dinov3");
    crate::visual::run(&mut f.repo, &mut job, &model, &cancel, |_| {}).unwrap();
    assert_eq!(f.repo.list_media().unwrap().len(), 1);
    assert_eq!(job.state, "completed");
}
