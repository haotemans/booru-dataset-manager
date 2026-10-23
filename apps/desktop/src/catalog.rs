use crate::{
    app::{BdmApp, Dialog},
    model::{Collection, Page},
};
use bdm_domain::catalog::{CollectionKind, CollectionRecord, SourceTag};
use bdm_ports::{CatalogError, CollectionRepository};
use eframe::egui;
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
};

struct PendingLookup {
    generation: u64,
    kind: CollectionKind,
    receiver: Receiver<Result<SourceTag, CatalogError>>,
}

#[derive(Default)]
pub struct CatalogSettings {
    pub repository: Option<Box<dyn CollectionRepository>>,
    pub path: Option<PathBuf>,
    pub error: Option<CatalogError>,
    pub kind: Option<CollectionKind>,
    pub found: Option<SourceTag>,
    pub lookup_error: Option<CatalogError>,
    pub edit_credentials: bool,
    pub anonymous: bool,
    pending: Option<PendingLookup>,
    generation: u64,
    capture_sender: Option<mpsc::Sender<Result<SourceTag, CatalogError>>>,
}

pub fn collection_kind(page: Page) -> Option<CollectionKind> {
    match page {
        Page::Artist => Some(CollectionKind::Artist),
        Page::Character => Some(CollectionKind::Character),
        _ => None,
    }
}

fn collection(record: &CollectionRecord) -> Result<Collection, CatalogError> {
    if let Some(tag) = &record.source {
        bdm_application::catalog::validate_tag(record.kind, tag)?;
    } else if record.kind != CollectionKind::Artist
        || record.name.is_empty()
        || record.name.len() > 512
        || record
            .name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(CatalogError::InvalidResponse);
    }
    Ok(Collection {
        id: usize::try_from(record.id)
            .ok()
            .filter(|id| *id > 0)
            .ok_or(CatalogError::InvalidResponse)?,
        tag: record.name.clone(),
        api_count: record
            .source
            .as_ref()
            .map(|tag| usize::try_from(tag.post_count))
            .transpose()
            .map_err(|_| CatalogError::InvalidResponse)?,
    })
}

impl BdmApp {
    pub fn capture_tag_state(&mut self, state: u8) {
        if !self.is_capture() {
            return;
        }
        self.catalog.pending = None;
        self.catalog.found = None;
        self.form_error.clear();
        self.catalog.lookup_error = None;
        match state {
            0 => {
                let (sender, receiver) = mpsc::channel();
                self.catalog.capture_sender = Some(sender);
                self.catalog.pending = Some(PendingLookup {
                    generation: self.catalog.generation,
                    kind: CollectionKind::Character,
                    receiver,
                });
            }
            1 => {
                self.catalog.capture_sender = None;
                self.form_error = CatalogError::NotFound.to_string();
            }
            3 => {
                self.catalog.capture_sender = None;
                self.catalog.lookup_error = Some(CatalogError::WebsiteVerification);
                self.form_error = CatalogError::WebsiteVerification.to_string();
            }
            4 => {
                self.catalog.capture_sender = None;
                self.danbooru.username = "synthetic_user".into();
                self.danbooru.saved_username = Some("synthetic_user".into());
                self.catalog.lookup_error = Some(CatalogError::Authentication);
                self.form_error = CatalogError::Authentication.to_string();
            }
            _ => {
                self.catalog.found = Some(SourceTag {
                    id: 41,
                    name: self.input_tag.clone(),
                    category: 4,
                    post_count: 120,
                })
            }
        }
    }
    pub fn open_catalog(&mut self, path: &Path) {
        if self.demo || self.is_capture() {
            return;
        }
        self.catalog.path = Some(path.to_owned());
        let result = bdm_infrastructure::catalog::SqliteCatalog::open(path).and_then(|mut repo| {
            repo.recover_imports().map_err(|_| CatalogError::Storage)?;
            repo.recover_repairs().map_err(|_| CatalogError::Storage)?;
            repo.recover_deletions()
                .map_err(|_| CatalogError::Storage)?;
            repo.attach_known_collections()
                .map_err(|_| CatalogError::Storage)?;
            repo.recover_import_tags()
                .map_err(|_| CatalogError::Storage)?;
            repo.materialize_artifacts()
                .map_err(|_| CatalogError::Storage)?;
            repo.recover_workflows()
                .map_err(|_| CatalogError::Storage)?;
            self.attach_catalog(Box::new(repo))
        });
        if result.is_ok() {
            if let Err(error) = self.reload_media() {
                self.ingest.error = Some(error);
            }
        }
        if let Err(error) = result {
            self.catalog.repository = None;
            self.catalog.error = Some(error);
            self.log("ERROR", &error.to_string());
        }
    }

    pub fn attach_catalog(
        &mut self,
        repo: Box<dyn CollectionRepository>,
    ) -> Result<(), CatalogError> {
        let records = bdm_application::catalog::load_collections(repo.as_ref())?;
        self.refresh_collection_records(records)?;
        self.catalog.repository = Some(repo);
        self.catalog.error = None;
        self.log("INFO", "已加载本地画师和角色记录。");
        Ok(())
    }

    pub(crate) fn refresh_collection_records(
        &mut self,
        records: Vec<CollectionRecord>,
    ) -> Result<(), CatalogError> {
        let mut artists = Vec::new();
        let mut characters = Vec::new();
        for record in records {
            let item = collection(&record)?;
            match record.kind {
                CollectionKind::Artist => artists.push(item),
                CollectionKind::Character => characters.push(item),
            }
        }
        artists.sort_by_key(|item| item.id);
        characters.sort_by_key(|item| item.id);
        if self.library.artists != artists || self.library.characters != characters {
            self.library.artists = artists;
            self.library.characters = characters;
            self.tags.invalidate();
        }
        Ok(())
    }

    pub fn begin_add_collection(&mut self) {
        self.cancel_tag_lookup();
        self.catalog.kind = collection_kind(self.prefs.page);
        self.input_tag.clear();
        self.form_error.clear();
        self.catalog.edit_credentials = false;
        self.catalog.anonymous = false;
        self.dialog = Some(Dialog::Add);
    }

    pub fn cancel_tag_lookup(&mut self) {
        self.catalog.generation = self.catalog.generation.wrapping_add(1);
        self.catalog.found = None;
        self.catalog.lookup_error = None;
    }

    pub fn tag_lookup_pending(&self) -> bool {
        self.catalog.pending.is_some()
    }

    pub fn start_tag_lookup(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.demo || self.is_capture() {
            return Err("示例预览不查询远程 Tag，也不写入本地收藏。".into());
        }
        if self.catalog.pending.is_some() {
            return Err("上一条查询正在结束，请稍后重试。".into());
        }
        if self.catalog.repository.is_none() {
            return Err(self
                .catalog
                .error
                .unwrap_or(CatalogError::Storage)
                .to_string());
        }
        let kind = self
            .catalog
            .kind
            .ok_or_else(|| CatalogError::Changed.to_string())?;
        let name =
            bdm_application::catalog::normalize_tag(&self.input_tag).map_err(|e| e.to_string())?;
        // Only explicitly saved credentials are sent. An unsaved account/key draft is never used.
        let credentials = self.tag_lookup_credentials().map_err(|error| {
            self.catalog.lookup_error = Some(error);
            error.to_string()
        })?;
        self.catalog.found = None;
        self.form_error.clear();
        self.catalog.lookup_error = None;
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name("danbooru-tag-lookup".into())
            .spawn(move || {
                let result = bdm_infrastructure::danbooru::DanbooruProvider::new(credentials)
                    .and_then(|provider| {
                        bdm_application::catalog::lookup_tag(&provider, kind, &name)
                    });
                let _ = sender.send(result);
                repaint.request_repaint();
            })
            .map_err(|_| CatalogError::Service.to_string())?;
        self.catalog.pending = Some(PendingLookup {
            generation: self.catalog.generation,
            kind,
            receiver,
        });
        Ok(())
    }

    fn tag_lookup_credentials(&self) -> Result<Option<bdm_ports::SourceCredentials>, CatalogError> {
        if self.catalog.anonymous {
            Ok(None)
        } else {
            self.saved_danbooru_credentials()
        }
    }

    pub fn poll_tag_lookup(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.catalog.pending else {
            return;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Disconnected) => Err(CatalogError::Service),
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
        };
        let pending = self.catalog.pending.take().expect("pending lookup");
        if pending.generation != self.catalog.generation
            || self.catalog.kind != Some(pending.kind)
            || !matches!(self.dialog, Some(Dialog::Add))
        {
            return;
        }
        self.catalog.found = None;
        self.catalog.lookup_error = result.as_ref().err().copied();
        match result {
            Ok(tag) => self.catalog.found = Some(tag),
            Err(error) => {
                self.form_error = error.to_string();
                self.log("WARN", &error.to_string());
            }
        }
    }

    pub fn save_found_collection(&mut self) -> Result<bool, String> {
        if self.demo || self.is_capture() {
            return Err("示例预览不写入本地收藏。".into());
        }
        let kind = self
            .catalog
            .kind
            .ok_or_else(|| CatalogError::Changed.to_string())?;
        let tag = self
            .catalog
            .found
            .as_ref()
            .ok_or_else(|| "请先查询 Tag。".to_owned())?;
        if bdm_application::catalog::normalize_tag(&self.input_tag).map_err(|e| e.to_string())?
            != tag.name
        {
            return Err("Tag 已修改，请重新查询。".into());
        }
        let repo = self
            .catalog
            .repository
            .as_mut()
            .ok_or_else(|| CatalogError::Storage.to_string())?;
        let result = bdm_application::catalog::add_collection(repo.as_mut(), kind, tag)
            .map_err(|e| e.to_string())?;
        let item = collection(&result.record).map_err(|e| e.to_string())?;
        let items = match kind {
            CollectionKind::Artist => &mut self.library.artists,
            CollectionKind::Character => &mut self.library.characters,
        };
        if let Some(old) = items.iter_mut().find(|old| old.id == item.id) {
            *old = item;
        } else {
            items.push(item);
            items.sort_by_key(|item| item.id);
        }
        let page = match kind {
            CollectionKind::Artist => Page::Artist,
            CollectionKind::Character => Page::Character,
        };
        self.navigate(page);
        if let Some(path) = &self.catalog.path {
            let mut media_repo = bdm_infrastructure::catalog::SqliteCatalog::open(path)
                .map_err(|e| e.to_string())?;
            let mut rules = media_repo.tag_rules().map_err(|e| e.to_string())?;
            rules
                .categories
                .insert(result.record.name.clone(), kind.category());
            media_repo
                .save_tag_rules(&rules)
                .map_err(|e| e.to_string())?;
            media_repo
                .attach_known_collections()
                .map_err(|e| e.to_string())?;
            self.reload_media()?;
        }
        self.log(
            "INFO",
            if result.created {
                "已将来源确认的 Tag 保存到本地收藏。"
            } else {
                "Tag 已在本地收藏中，未创建重复记录。"
            },
        );
        Ok(result.created)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{collections::CollectionDeletion, model::Preferences};
    use std::collections::BTreeSet;

    fn app() -> BdmApp {
        let mut app = BdmApp::from_preferences(Preferences::default(), false, None);
        app.attach_catalog(Box::new(
            bdm_infrastructure::catalog::SqliteCatalog::in_memory().unwrap(),
        ))
        .unwrap();
        app.navigate(Page::Character);
        app.begin_add_collection();
        app
    }
    fn tag(id: i64) -> SourceTag {
        SourceTag {
            id,
            name: format!("synthetic_character_{id}"),
            category: 4,
            post_count: 120,
        }
    }
    fn found(app: &mut BdmApp, id: i64) {
        app.input_tag = tag(id).name.clone();
        app.catalog.found = Some(tag(id));
    }
    fn pending(app: &mut BdmApp) -> mpsc::Sender<Result<SourceTag, CatalogError>> {
        let (sender, receiver) = mpsc::channel();
        app.catalog.pending = Some(PendingLookup {
            generation: app.catalog.generation,
            kind: CollectionKind::Character,
            receiver,
        });
        sender
    }

    #[test]
    fn add_duplicate_and_restart_keep_identity_counts_and_order() {
        let path = std::env::temp_dir().join(format!(
            "bdm-角色 收藏-{}.sqlite3",
            crate::model::new_profile_id()
        ));
        let first_id;
        {
            let mut app = app();
            app.open_catalog(&path);
            found(&mut app, 41);
            assert_eq!(app.save_found_collection(), Ok(true));
            first_id = app.library.characters[0].id;
            app.begin_add_collection();
            found(&mut app, 41);
            assert_eq!(app.save_found_collection(), Ok(false));
            app.begin_add_collection();
            found(&mut app, 42);
            assert_eq!(app.save_found_collection(), Ok(true));
            assert_eq!(app.library.characters.len(), 2);
            assert!(app.library.characters[1].id > first_id);
            assert!(app.library.media.is_empty());
        }
        {
            let mut app = app();
            app.open_catalog(&path);
            assert_eq!(app.library.characters[0].id, first_id);
            assert_eq!(app.library.characters[0].api_count, Some(120));
            let scope = app
                .collection_deletion_scope(Page::Character, &BTreeSet::from([first_id]))
                .unwrap();
            assert_eq!(app.confirm_collection_deletion(&scope), Ok((1, 0)));
        }
        {
            let mut app = app();
            app.open_catalog(&path);
            assert_eq!(app.library.characters.len(), 1);
            assert_eq!(app.library.characters[0].tag, tag(42).name);
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn errors_edits_and_wrong_categories_preserve_input_and_records() {
        let mut app = app();
        found(&mut app, 41);
        app.input_tag = "edited".into();
        assert!(app.save_found_collection().is_err());
        assert!(app.library.characters.is_empty());
        found(&mut app, 41);
        app.catalog.found.as_mut().unwrap().category = 1;
        assert!(app.save_found_collection().is_err());
        let sender = pending(&mut app);
        sender.send(Err(CatalogError::NotFound)).unwrap();
        app.poll_tag_lookup(&egui::Context::default());
        assert_eq!(app.input_tag, tag(41).name);
        assert_eq!(app.form_error, CatalogError::NotFound.to_string());
        assert!(app.library.characters.is_empty());
        assert!(app
            .catalog
            .repository
            .as_ref()
            .unwrap()
            .list()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn downloaded_character_artists_refresh_without_restart_and_open_the_shared_image() {
        use bdm_ports::{IngestRepository, MediaProvider};
        use md5::{Digest, Md5};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog.sqlite3");
        let mut app = app();
        app.open_catalog(&path);
        let mut repo = bdm_infrastructure::catalog::SqliteCatalog::open(&path).unwrap();
        let character = repo
            .add(CollectionKind::Character, &tag(41))
            .unwrap()
            .record
            .id;
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            32,
            24,
            image::Rgb([10, 30, 50]),
        ))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
        let bytes = bytes.into_inner();
        let post = bdm_domain::ingest::SourcePost {
            id: 100,
            md5: format!("{:x}", Md5::digest(&bytes)),
            file_url: Some("https://cdn.donmai.us/fixture.png".into()),
            file_ext: "png".into(),
            file_size: bytes.len() as u64,
            image_width: 32,
            image_height: 24,
            tag_string: "artist collaborator synthetic_character_41".into(),
            tag_string_artist: "artist collaborator".into(),
            tag_string_character: "synthetic_character_41".into(),
            tag_string_general: String::new(),
            tag_string_copyright: String::new(),
            tag_string_meta: String::new(),
            rating: "g".into(),
            is_deleted: false,
        };
        struct Provider(Vec<u8>);
        impl MediaProvider for Provider {
            fn posts(
                &self,
                _: &str,
                _: Option<i64>,
            ) -> Result<Vec<bdm_domain::ingest::SourcePost>, bdm_ports::IngestError> {
                Ok(Vec::new())
            }
            fn download(
                &self,
                _: &bdm_domain::ingest::SourcePost,
                path: &Path,
                _: &std::sync::atomic::AtomicBool,
            ) -> Result<(), bdm_ports::IngestError> {
                std::fs::write(path, &self.0).map_err(|_| bdm_ports::IngestError::Storage)
            }
        }
        repo.import(
            &post,
            &directory.path().join("library"),
            character,
            1,
            &std::sync::atomic::AtomicBool::new(false),
            &Provider(bytes),
        )
        .unwrap();
        let job = repo
            .new_job(
                character,
                "synthetic_character_41".into(),
                directory.path().join("library"),
                1,
            )
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        app.pending_for_test(receiver);
        sender.send(job.clone()).unwrap();
        app.poll_ingest(&egui::Context::default());
        assert_eq!(app.library.artists.len(), 2);
        assert!(app
            .library
            .artists
            .iter()
            .all(|artist| artist.api_count.is_none()));
        let media_id = app.library.media[0].id;
        for artist in &app.library.artists {
            assert_eq!(
                app.associated_media(Page::Artist, &BTreeSet::from([artist.id]))[0].id,
                media_id
            );
            assert_eq!(
                app.ingest.collection_summaries[&artist.id],
                (1, Some(media_id))
            );
        }
        // A burst updates progress immediately while deferring repeated full-library loads.
        repo.add(
            CollectionKind::Artist,
            &SourceTag {
                id: 42,
                name: "later_artist".into(),
                category: 1,
                post_count: 12,
            },
        )
        .unwrap();
        sender.send(job).unwrap();
        app.poll_ingest(&egui::Context::default());
        assert_eq!(app.library.artists.len(), 2);
        drop(sender);
        app.poll_ingest(&egui::Context::default());
        assert_eq!(app.library.artists.len(), 3);
        app.navigate(Page::Artist);
        app.context = Some(app.library.artists[0].id);
        app.gallery = true;
        assert_eq!(app.scope_media()[0].id, media_id);
        app.open_media(media_id);
        assert!(matches!(app.dialog,Some(Dialog::Media(id)) if id == media_id));
        let mut reopened = BdmApp::from_preferences(Preferences::default(), false, None);
        reopened.open_catalog(&path);
        assert_eq!(reopened.library.artists, app.library.artists);
        assert_eq!(
            reopened.ingest.collection_summaries[&reopened.library.artists[0].id],
            (1, Some(media_id))
        );
        let artist_id = reopened.library.artists[0].id;
        reopened.remove_sample_media(&BTreeSet::from([media_id]));
        assert!(!reopened
            .ingest
            .collection_summaries
            .contains_key(&artist_id));
        assert!(reopened
            .associated_media(Page::Artist, &BTreeSet::from([artist_id]))
            .is_empty());
    }

    #[test]
    fn cancelled_or_old_results_never_populate_a_new_dialog() {
        let mut app = app();
        let sender = pending(&mut app);
        app.navigate(Page::Artist);
        app.begin_add_collection();
        sender.send(Ok(tag(41))).unwrap();
        app.poll_tag_lookup(&egui::Context::default());
        assert!(app.catalog.found.is_none());
        assert!(!app.tag_lookup_pending());
        app.navigate(Page::Character);
        app.begin_add_collection();
        let sender = pending(&mut app);
        sender.send(Ok(tag(42))).unwrap();
        app.poll_tag_lookup(&egui::Context::default());
        assert_eq!(app.catalog.found, Some(tag(42)));
        assert!(app.library.characters.is_empty());
    }

    #[test]
    fn storage_failure_and_nonempty_deletion_do_not_mutate_visible_records() {
        struct Failed;
        impl CollectionRepository for Failed {
            fn list(&self) -> Result<Vec<CollectionRecord>, CatalogError> {
                Ok(vec![])
            }
            fn add(
                &mut self,
                _: CollectionKind,
                _: &SourceTag,
            ) -> Result<bdm_domain::catalog::AddCollectionResult, CatalogError> {
                Err(CatalogError::Storage)
            }
            fn delete_empty(&mut self, _: CollectionKind, _: &[i64]) -> Result<(), CatalogError> {
                Err(CatalogError::Storage)
            }
        }
        let mut app = app();
        found(&mut app, 41);
        app.catalog.repository = Some(Box::new(Failed));
        assert!(app.save_found_collection().is_err());
        assert!(app.library.characters.is_empty());
        assert!(app.catalog.found.is_some());
        app.library.characters.push(Collection {
            id: 1,
            tag: "keep".into(),
            api_count: Some(0),
        });
        let scope = CollectionDeletion {
            page: Page::Character,
            ids: BTreeSet::from([1]),
            media_ids: BTreeSet::new(),
        };
        assert!(app.confirm_collection_deletion(&scope).is_err());
        assert_eq!(app.library.characters.len(), 1);
        let nonempty = CollectionDeletion {
            media_ids: BTreeSet::from([99]),
            ..scope
        };
        assert!(!app.can_delete_empty_collection_scope(&nonempty));
        assert!(app.confirm_collection_deletion(&nonempty).is_err());
    }

    #[test]
    fn preview_capture_and_missing_database_gate_all_real_work() {
        let ctx = egui::Context::default();
        for (demo, capture) in [
            (true, None),
            (false, Some(PathBuf::from("target/isolated"))),
        ] {
            let mut app = BdmApp::from_preferences(Preferences::default(), demo, capture);
            app.navigate(Page::Character);
            app.begin_add_collection();
            found(&mut app, 41);
            assert!(app.start_tag_lookup(&ctx).is_err());
            assert!(app.save_found_collection().is_err());
        }
        let mut app = app();
        app.catalog.repository = None;
        assert!(app.start_tag_lookup(&ctx).is_err());
    }

    #[test]
    fn explicit_anonymous_recovery_never_reads_or_changes_saved_credentials() {
        struct Unreadable;
        impl bdm_ports::SourceCredentialStore for Unreadable {
            fn load(
                &self,
            ) -> Result<Option<bdm_ports::SourceCredentials>, bdm_ports::CredentialError>
            {
                panic!("anonymous recovery must not read credentials")
            }
            fn save(
                &self,
                _: &bdm_ports::SourceCredentials,
            ) -> Result<(), bdm_ports::CredentialError> {
                panic!("anonymous recovery must not save credentials")
            }
            fn clear(&self) -> Result<(), bdm_ports::CredentialError> {
                panic!("anonymous recovery must not delete credentials")
            }
        }
        let mut app = app();
        app.danbooru.saved_username = Some("synthetic_user".into());
        app.danbooru.store = Box::new(Unreadable);
        app.input_tag = "phoenix_reffaella".into();
        app.catalog.anonymous = true;
        assert!(app.tag_lookup_credentials().unwrap().is_none());
        assert_eq!(
            app.danbooru.saved_username.as_deref(),
            Some("synthetic_user")
        );
        app.begin_add_collection();
        assert!(!app.catalog.anonymous);
    }

    #[test]
    fn credential_recovery_keeps_input_and_discards_previous_lookup() {
        let mut app = app();
        app.input_tag = "phoenix_reffaella".into();
        let sender = pending(&mut app);
        app.cancel_tag_lookup();
        sender.send(Ok(tag(41))).unwrap();
        app.poll_tag_lookup(&egui::Context::default());
        assert!(app.catalog.found.is_none());
        assert_eq!(app.catalog.kind, Some(CollectionKind::Character));
        assert_eq!(app.input_tag, "phoenix_reffaella");
        assert!(matches!(app.dialog, Some(Dialog::Add)));
    }

    #[test]
    fn authentication_recovery_actions_render_and_escape_clears_key_draft() {
        fn texts(shape: &egui::epaint::Shape) -> Vec<&egui::epaint::TextShape> {
            match shape {
                egui::epaint::Shape::Text(text) => vec![text],
                egui::epaint::Shape::Vec(shapes) => shapes.iter().flat_map(texts).collect(),
                _ => vec![],
            }
        }
        for size in [[981.0, 613.0], [1050.0, 720.0]] {
            let mut app = app();
            app.input_tag = "phoenix_reffaella".into();
            let sender = pending(&mut app);
            sender.send(Err(CatalogError::Authentication)).unwrap();
            let ctx = egui::Context::default();
            crate::theme::setup(&ctx);
            app.poll_tag_lookup(&ctx);
            app.catalog.edit_credentials = true;
            app.danbooru.api_key = "synthetic-secret".into();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size[0], size[1]),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input.clone(), |ctx| app.shell(ctx));
            let output = ctx.run(input, |ctx| app.shell(ctx));
            let shapes: Vec<_> = output
                .shapes
                .iter()
                .flat_map(|shape| texts(&shape.shape))
                .collect();
            for label in ["修改凭据", "不使用账号重试", "保存并重试"] {
                let shape = shapes
                    .iter()
                    .find(|shape| shape.galley.job.text == label)
                    .unwrap();
                assert!(shape.pos.x >= 0.0 && shape.pos.x + shape.galley.rect.width() <= size[0]);
                assert!(shape.pos.y >= 0.0 && shape.pos.y + shape.galley.rect.height() <= size[1]);
            }
            assert!(!shapes
                .iter()
                .any(|shape| shape.galley.job.text.contains("synthetic-secret")));
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ctx| app.show_dialog(ctx),
            );
            assert!(app.danbooru.api_key.is_empty());
            assert!(app.dialog.is_none());
            assert!(app.library.characters.is_empty());
            app.begin_add_collection();
            app.catalog.edit_credentials = true;
            app.danbooru.api_key = "navigation-secret".into();
            app.navigate(Page::Settings);
            assert!(app.danbooru.api_key.is_empty());
            assert!(!app.catalog.edit_credentials);
        }
    }

    #[test]
    fn website_verification_preserves_tag_and_wraps_recovery_prompt() {
        fn texts(shape: &egui::epaint::Shape) -> Vec<&egui::epaint::TextShape> {
            match shape {
                egui::epaint::Shape::Text(text) => vec![text],
                egui::epaint::Shape::Vec(shapes) => shapes.iter().flat_map(texts).collect(),
                _ => vec![],
            }
        }
        for size in [[981.0, 613.0], [1050.0, 720.0]] {
            let mut app = app();
            app.input_tag = "phoenix_reffaella".into();
            let sender = pending(&mut app);
            sender.send(Err(CatalogError::WebsiteVerification)).unwrap();
            let ctx = egui::Context::default();
            crate::theme::setup(&ctx);
            app.poll_tag_lookup(&ctx);
            assert_eq!(app.input_tag, "phoenix_reffaella");
            assert_eq!(
                app.catalog.lookup_error,
                Some(CatalogError::WebsiteVerification)
            );
            assert!(app
                .catalog
                .repository
                .as_ref()
                .unwrap()
                .list()
                .unwrap()
                .is_empty());
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size[0], size[1]),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input.clone(), |ctx| app.shell(ctx));
            let output = ctx.run(input, |ctx| app.shell(ctx));
            let shapes: Vec<_> = output
                .shapes
                .iter()
                .flat_map(|shape| texts(&shape.shape))
                .collect();
            let error = shapes
                .iter()
                .find(|shape| {
                    shape.galley.job.text == CatalogError::WebsiteVerification.to_string()
                })
                .unwrap();
            assert!(error.galley.rows.len() > 1, "long error should wrap");
            assert!(error.galley.rect.width() <= 461.0);
            assert!(error.pos.x >= 0.0 && error.pos.x + error.galley.rect.width() <= size[0]);
            assert!(shapes
                .iter()
                .any(|shape| shape.galley.job.text == "在浏览器中检查 Danbooru"));
        }
    }

    #[test]
    fn add_dialog_renders_at_both_sizes_and_escape_discards_pending_result() {
        for size in [[1050.0, 720.0], [1440.0, 960.0]] {
            let mut app = app();
            found(&mut app, 41);
            let ctx = egui::Context::default();
            crate::theme::setup(&ctx);
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(size[0], size[1]),
                    )),
                    ..Default::default()
                },
                |ctx| app.shell(ctx),
            );
            let sender = pending(&mut app);
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ctx| app.show_dialog(ctx),
            );
            sender.send(Ok(tag(42))).unwrap();
            app.poll_tag_lookup(&ctx);
            assert!(app.dialog.is_none());
            assert!(app.catalog.found.is_none());
            assert!(app.library.characters.is_empty());
        }
    }
}
