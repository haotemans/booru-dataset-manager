use crate::app::BdmApp;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct AgentSync {
    last_poll: Option<Instant>,
    revision: i64,
    queued: BTreeSet<usize>,
}

impl BdmApp {
    pub(crate) fn poll_agent_changes(&mut self, ctx: &eframe::egui::Context) {
        if self.demo || self.is_capture() {
            return;
        }
        ctx.request_repaint_after(Duration::from_secs(1));
        if self.ingest_active()
            || self.workflow_active()
            || self.image_editor.active()
            || self.image_editor.visible
            || self
                .agent_sync
                .last_poll
                .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.agent_sync.last_poll = Some(Instant::now());
        let Some(path) = self.catalog.path.as_deref() else {
            return;
        };
        let Ok(state) = bdm_infrastructure::agent::snapshot(path) else {
            return;
        };
        if state.revision == self.agent_sync.revision {
            return;
        }
        if let Err(error) = self.reload_media() {
            self.form_error = error;
            return;
        }
        for id in &self.agent_sync.queued {
            self.pending_deletion.remove(id);
        }
        self.pending_deletion.extend(state.rejected.iter());
        self.agent_sync.queued = state.rejected;
        self.agent_sync.revision = state.revision;
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdm_domain::{
        catalog::{CollectionKind, SourceTag},
        editing::CropRect,
        ingest::{SourcePost, StoredMedia},
        sources::SourceProvider,
    };
    use bdm_infrastructure::{catalog::SqliteCatalog, editing};
    use bdm_ports::{CollectionRepository, IngestError, IngestRepository, MediaProvider};
    use eframe::egui;
    use md5::{Digest, Md5};
    use std::{path::Path, sync::atomic::AtomicBool};

    struct Provider(Vec<u8>);
    impl MediaProvider for Provider {
        fn posts(&self, _: &str, _: Option<i64>) -> Result<Vec<SourcePost>, IngestError> {
            Ok(Vec::new())
        }
        fn download(&self, _: &SourcePost, path: &Path, _: &AtomicBool) -> Result<(), IngestError> {
            std::fs::write(path, &self.0).map_err(|_| IngestError::Storage)
        }
    }
    struct Fixture {
        app: BdmApp,
        repo: SqliteCatalog,
        media: StoredMedia,
        ctx: egui::Context,
        _directory: tempfile::TempDir,
    }
    fn fixture() -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog.sqlite3");
        let mut repo = SqliteCatalog::open(&path).unwrap();
        let collection = repo
            .add(
                CollectionKind::Artist,
                &SourceTag {
                    provider: SourceProvider::Danbooru,
                    id: 1,
                    name: "sync_artist".into(),
                    category: 1,
                    post_count: 1,
                },
            )
            .unwrap()
            .record
            .id;
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(1024, 800, image::Rgb([30, 60, 90]))
            .write_to(&mut bytes, image::ImageFormat::Jpeg)
            .unwrap();
        let provider = Provider(bytes.into_inner());
        let post = SourcePost {
            provider: SourceProvider::Danbooru,
            id: 99,
            md5: format!("{:x}", Md5::digest(&provider.0)),
            file_url: Some("https://cdn.danbooru.donmai.us/original/test.jpg".into()),
            file_ext: "jpg".into(),
            file_size: provider.0.len() as u64,
            image_width: 1024,
            image_height: 800,
            tag_string: "sync_artist blue".into(),
            tag_string_artist: "sync_artist".into(),
            tag_string_character: String::new(),
            tag_string_general: "blue".into(),
            tag_string_copyright: String::new(),
            tag_string_meta: String::new(),
            rating: "g".into(),
            is_deleted: false,
        };
        repo.import(
            &post,
            &directory.path().join("library"),
            collection,
            800,
            &AtomicBool::new(false),
            &provider,
        )
        .unwrap();
        let media = repo.list_media().unwrap().remove(0);
        let mut app = BdmApp::from_preferences(crate::model::Preferences::default(), false, None);
        app.catalog.path = Some(path);
        app.reload_media().unwrap();
        app.open_media(media.id as usize);
        let ctx = egui::Context::default();
        app.poll_agent_changes(&ctx);
        Fixture {
            app,
            repo,
            media,
            ctx,
            _directory: directory,
        }
    }
    fn external_crop(f: &mut Fixture) -> StoredMedia {
        let preview = editing::native_crop_preview(
            &f.media,
            CropRect {
                x: 4,
                y: 4,
                width: 1016,
                height: 792,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        f.repo.commit_repair(f.media.id, &preview).unwrap()
    }
    fn loaded_size(f: &mut Fixture) -> [usize; 2] {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(texture) = f.app.review_texture(&f.ctx, f.media.id as usize) {
                return texture.size();
            }
            assert!(
                f.app.ingest.detail_error.is_none(),
                "{:?}",
                f.app.ingest.detail_error
            );
            assert!(Instant::now() < deadline, "Image decode never completed");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn native_external_crop_refreshes_path_and_failed_texture_without_agent_audit() {
        let mut f = fixture();
        let id = f.media.id as usize;
        f.app.load_review_original(&f.ctx, id);
        assert_eq!(loaded_size(&mut f), [1024, 800]);
        let changed = external_crop(&mut f);
        assert!(!f.media.path.exists());
        assert!(changed.path.is_file());
        f.app.ingest.detail_texture = None;
        f.app.ingest.detail_error = Some((id, "Unable to read old JPEG path".into()));
        f.app.agent_sync.last_poll = None;
        f.app.poll_agent_changes(&f.ctx);
        assert_eq!(f.app.ingest.stored[&id].path, changed.path);
        assert!(f.app.ingest.detail_error.is_none());
        assert_eq!(loaded_size(&mut f), [992, 768]);
        assert!(matches!(f.app.dialog,Some(crate::app::Dialog::Media(current)) if current==id));
        // Reloading is read-only and must not create a stream of new revisions.
        let revision = bdm_infrastructure::agent::snapshot(f.app.catalog.path.as_deref().unwrap())
            .unwrap()
            .revision;
        f.app.agent_sync.last_poll = None;
        f.app.poll_agent_changes(&f.ctx);
        assert_eq!(
            revision,
            bdm_infrastructure::agent::snapshot(f.app.catalog.path.as_deref().unwrap())
                .unwrap()
                .revision
        );
    }
    #[test]
    fn retry_refreshes_stale_jpeg_path_before_decoder_and_local_edit_defers_polling() {
        let mut f = fixture();
        let id = f.media.id as usize;
        let changed = external_crop(&mut f);
        f.app.image_editor.visible = true;
        f.app.agent_sync.last_poll = None;
        f.app.poll_agent_changes(&f.ctx);
        assert_eq!(f.app.ingest.stored[&id].path, f.media.path);
        f.app.image_editor.visible = false;
        // Retry heals the stale path even before the periodic poll runs.
        f.app.load_review_original(&f.ctx, id);
        assert_eq!(f.app.ingest.stored[&id].path, changed.path);
        assert_eq!(loaded_size(&mut f), [992, 768]);
    }
}
