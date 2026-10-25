use crate::{
    app::BdmApp,
    model::{Media, Page},
    theme::*,
    widgets::*,
};
use bdm_domain::ingest::{IngestJob, StoredMedia};
use eframe::egui::{self, vec2, RichText};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
};

#[cfg(test)]
mod tests {
    use super::*;
    use bdm_domain::ingest::SourcePost;

    #[test]
    fn review_defaults_to_thumbnail_and_original_requires_explicit_loading() {
        let mut app = BdmApp::from_preferences(crate::model::Preferences::default(), false, None);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("original.png");
        let thumbnail = directory.path().join("thumbnail.png");
        image::RgbImage::from_pixel(80, 60, image::Rgb([30, 60, 90]))
            .save(&path)
            .unwrap();
        image::RgbImage::from_pixel(16, 12, image::Rgb([30, 60, 90]))
            .save(&thumbnail)
            .unwrap();
        app.ingest.stored.insert(
            1,
            StoredMedia {
                id: 1,
                path: path.clone(),
                thumbnail,
                sha256: String::new(),
                source_sha256: String::new(),
                collections: BTreeSet::new(),
                keep: false,
                minimum_short_side: 1,
                flatten_transparency: true,
                edit: None,
                post: SourcePost {
                    id: 100,
                    md5: String::new(),
                    file_url: None,
                    file_ext: "png".into(),
                    file_size: 1,
                    image_width: 80,
                    image_height: 60,
                    tag_string: String::new(),
                    tag_string_artist: String::new(),
                    tag_string_character: String::new(),
                    tag_string_general: String::new(),
                    tag_string_copyright: String::new(),
                    tag_string_meta: String::new(),
                    rating: "g".into(),
                    is_deleted: false,
                },
            },
        );
        let media = Media {
            id: 1,
            artist: None,
            character: None,
            tags: Vec::new(),
            keep: false,
            optimized: false,
            caption: false,
            width: 80,
            height: 60,
            rating: 1,
        };
        app.library.media = vec![media.clone(), Media { id: 2, ..media }];
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        app.open_media(1);
        for _ in 0..3 {
            review_frame(&ctx, &mut app, Vec::new());
        }
        let thumbnail_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !app.ingest.textures.contains_key(&1) {
            app.ingest.thumbnails.poll(&ctx, &mut app.ingest.textures);
            review_frame(&ctx, &mut app, Vec::new());
            assert!(std::time::Instant::now() < thumbnail_deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(app.review_texture(&ctx, 1).is_none());
        assert!(app.ingest.detail_requested.is_none());
        assert!(app.ingest.detail_pending.is_none());
        assert!(app.ingest.detail_error.is_none());
        assert_eq!(app.ingest.textures[&1].size(), [16, 12]);
        click_original_control(&ctx, &mut app);
        assert_eq!(app.ingest.detail_requested, Some(1));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let texture = loop {
            if let Some(texture) = app.review_texture(&ctx, 1) {
                break texture;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(texture.size(), [80, 60]);
        assert!(app.review_texture(&ctx, 2).is_none());
        // Returning to thumbnails releases the original; polling cannot start it again.
        click_original_control(&ctx, &mut app);
        assert!(app.ingest.detail_texture.is_none());
        assert!(app.review_texture(&ctx, 1).is_none());
        assert!(app.ingest.detail_pending.is_none());

        // A late decode reply is discarded when navigation changes the current image.
        let (sender, receiver) = mpsc::channel();
        app.ingest.detail_requested = Some(1);
        app.ingest.detail_pending = Some((1, receiver));
        app.dialog = Some(crate::app::Dialog::Media(2));
        review_frame(&ctx, &mut app, Vec::new());
        assert!(sender.send(Ok(egui::ColorImage::example())).is_err());
        assert!(app.ingest.detail_requested.is_none());
        assert!(app.ingest.detail_texture.is_none());
        app.open_media(1);
        review_frame(&ctx, &mut app, Vec::new());
        assert!(app.ingest.detail_pending.is_none());

        std::fs::write(&path, b"invalid image").unwrap();
        assert!(app.review_texture(&ctx, 1).is_none());
        assert!(
            app.ingest.detail_error.is_none(),
            "browsing must not decode even a corrupt original"
        );
        click_original_control(&ctx, &mut app);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.ingest.detail_error.is_none() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
            app.review_texture(&ctx, 1);
        }
        assert!(app.review_texture(&ctx, 1).is_none());
        assert!(
            app.ingest.detail_pending.is_none(),
            "failed images must wait for explicit retry"
        );
        image::RgbImage::from_pixel(80, 60, image::Rgb([30, 60, 90]))
            .save(&path)
            .unwrap();
        app.load_review_original(&ctx, 1);
        while app.review_texture(&ctx, 1).is_none() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        review_frame(
            &ctx,
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(app.dialog.is_none());
        assert!(app.ingest.detail_requested.is_none());
        assert!(app.ingest.detail_texture.is_none());
        app.open_media(1);
        review_frame(&ctx, &mut app, Vec::new());
        assert!(app.review_texture(&ctx, 1).is_none());
        assert!(app.ingest.detail_pending.is_none());

        // The saved default applies once per image; a manual thumbnail override survives idle frames.
        toggle_original_setting(&ctx, &mut app);
        assert!(app.prefs.auto_load_review_original);
        app.open_media(1);
        review_frame(&ctx, &mut app, Vec::new());
        assert_eq!(app.ingest.detail_requested, Some(1));
        click_original_control(&ctx, &mut app);
        for _ in 0..3 {
            review_frame(&ctx, &mut app, Vec::new());
        }
        assert!(app.ingest.detail_requested.is_none());
        assert!(app.ingest.detail_pending.is_none());
        let mut second = app.ingest.stored[&1].clone();
        second.id = 2;
        app.ingest.stored.insert(2, second);
        app.dialog = Some(crate::app::Dialog::Media(2));
        review_frame(&ctx, &mut app, Vec::new());
        assert_eq!(app.ingest.detail_requested, Some(2));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.review_texture(&ctx, 2).is_none() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        toggle_original_setting(&ctx, &mut app);
        assert!(!app.prefs.auto_load_review_original);
        app.open_media(1);
        review_frame(&ctx, &mut app, Vec::new());
        assert!(app.ingest.detail_requested.is_none());
        assert!(app.ingest.detail_pending.is_none());
    }

    fn toggle_original_setting(ctx: &egui::Context, app: &mut BdmApp) {
        let frame = |app: &mut BdmApp, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(1050.0, 720.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.settings_page(ui));
                },
            )
        };
        for _ in 0..3 {
            frame(app, Vec::new());
        }
        let output = frame(app, Vec::new());
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == "审核时自动加载原图" => {
                    Some(t.pos + vec2(20.0, 8.0))
                }
                _ => None,
            })
            .unwrap();
        for pressed in [true, false] {
            frame(
                app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
    }

    fn review_frame(ctx: &egui::Context, app: &mut BdmApp, events: Vec<egui::Event>) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(1050.0, 720.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.show_dialog(ctx),
        );
    }

    fn click_original_control(ctx: &egui::Context, app: &mut BdmApp) {
        let pos = app.media_review.load_original_button.unwrap().center();
        for pressed in [true, false] {
            review_frame(
                ctx,
                app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
    }

    #[test]
    fn active_download_keeps_text_work_manual() {
        let mut app = BdmApp::from_preferences(crate::model::Preferences::default(), false, None);
        let path =
            std::env::temp_dir().join(format!("bdm-ingest-gate-{}.sqlite3", std::process::id()));
        app.catalog.path = Some(path.clone());
        let (_sender, receiver) = mpsc::channel();
        app.ingest.pending = Some(receiver);
        let ctx = egui::Context::default();
        app.start_background_analysis(&ctx);
        assert!(!app.workflow_active());
        let error = app
            .start_workflow(&ctx, "optimize", vec![1], None)
            .unwrap_err();
        assert!(error.contains("正在下载"));
        assert!(app.workflows.jobs.is_empty());
        std::fs::remove_file(path).unwrap();
    }
}

#[derive(Default)]
pub struct IngestSettings {
    pub jobs: Vec<IngestJob>,
    pub stored: BTreeMap<usize, StoredMedia>,
    pub textures: BTreeMap<usize, egui::TextureHandle>,
    pub thumbnails: crate::thumbnails::Thumbnails,
    pub collection_summaries: BTreeMap<usize, (usize, Option<usize>)>,
    pub error: Option<String>,
    pub detail_requested: Option<usize>,
    pub detail_texture: Option<(usize, egui::TextureHandle)>,
    pub detail_error: Option<(usize, String)>,
    detail_pending: Option<(usize, Receiver<Result<egui::ColorImage, String>>)>,
    pending: Option<Receiver<IngestJob>>,
    cancel: Option<Arc<AtomicBool>>,
    refresh_pending: bool,
    last_refresh: Option<std::time::Instant>,
}

impl BdmApp {
    pub(crate) fn clear_review_original(&mut self) {
        self.ingest.detail_requested = None;
        self.ingest.detail_texture = None;
        self.ingest.detail_error = None;
        self.ingest.detail_pending = None;
    }

    pub(crate) fn load_review_original(&mut self, ctx: &egui::Context, id: usize) {
        if self.demo || self.is_capture() || !self.ingest.stored.contains_key(&id) {
            return;
        }
        self.clear_review_original();
        self.ingest.detail_requested = Some(id);
        self.review_texture(ctx, id);
    }

    pub(crate) fn invalidate_review_texture(&mut self, id: usize) {
        self.ingest.textures.remove(&id);
        self.ingest.thumbnails.invalidate(id);
        if self.ingest.detail_requested == Some(id) {
            self.ingest.detail_texture = None;
            self.ingest.detail_error = None;
            self.ingest.detail_pending = None;
        }
    }
    #[cfg(test)]
    pub(crate) fn pending_for_test(&mut self, receiver: Receiver<IngestJob>) {
        self.ingest.pending = Some(receiver);
    }
    pub fn reload_media(&mut self) -> Result<(), String> {
        if self.demo || self.is_capture() {
            return Ok(());
        }
        let path = self.catalog.path.clone().ok_or("本地数据库未打开。")?;
        let repo =
            bdm_infrastructure::catalog::SqliteCatalog::open(&path).map_err(|e| e.to_string())?;
        let stored = repo.list_media().map_err(|e| e.to_string())?;
        let records = bdm_ports::CollectionRepository::list(&repo).map_err(|e| e.to_string())?;
        self.refresh_collection_records(records)
            .map_err(|e| e.to_string())?;
        if stored.iter().any(|m| !m.path.is_file()) {
            self.ingest.error = Some("部分原图文件缺失，请检查托管目录；元数据仍已保留。".into());
        }
        self.ingest.jobs = repo.list_jobs().map_err(|e| e.to_string())?;
        self.library.media = stored
            .iter()
            .map(|item| {
                let artist = self
                    .library
                    .artists
                    .iter()
                    .find(|c| item.collections.contains(&(c.id as i64)))
                    .map(|c| c.id);
                let character = self
                    .library
                    .characters
                    .iter()
                    .find(|c| item.collections.contains(&(c.id as i64)))
                    .map(|c| c.id);
                Media {
                    id: item.id as usize,
                    artist,
                    character,
                    tags: item
                        .post
                        .tag_string
                        .split_whitespace()
                        .map(str::to_owned)
                        .collect(),
                    keep: item.keep,
                    optimized: false,
                    caption: false,
                    width: item.dimensions().0 as usize,
                    height: item.dimensions().1 as usize,
                    rating: 1,
                }
            })
            .collect();
        let stored: BTreeMap<usize, StoredMedia> =
            stored.into_iter().map(|m| (m.id as usize, m)).collect();
        if self.ingest.stored != stored {
            let invalid: Vec<_> = self
                .ingest
                .stored
                .iter()
                .filter(|(id, old)| {
                    stored.get(id).is_none_or(|new| {
                        new.thumbnail != old.thumbnail
                            || new.sha256 != old.sha256
                            || new.path != old.path
                            || new.edit != old.edit
                    })
                })
                .map(|(id, _)| *id)
                .collect();
            for id in invalid {
                self.invalidate_review_texture(id);
            }
            self.tags.invalidate();
            self.ingest.stored = stored;
            self.rebuild_collection_summaries();
        }
        let valid: BTreeSet<_> = self.ingest.stored.keys().copied().collect();
        self.selected_media.retain(|id| valid.contains(id));
        self.pending_deletion.retain(|id| valid.contains(id));
        self.review_order.retain(|id| valid.contains(id));
        self.action_snapshot.retain(|id| valid.contains(id));
        if matches!(self.dialog,Some(crate::app::Dialog::Media(id)) if !valid.contains(&id)) {
            self.dialog = None;
        }
        self.reload_workflow_data()?;
        Ok(())
    }

    pub fn associated_media(&self, page: Page, ids: &BTreeSet<usize>) -> Vec<&Media> {
        if self.demo {
            return self.library.associated(page, ids);
        }
        self.library
            .media
            .iter()
            .filter(|m| {
                self.ingest.stored.get(&m.id).is_some_and(|item| {
                    item.collections
                        .iter()
                        .any(|id| ids.contains(&(*id as usize)))
                })
            })
            .collect()
    }

    pub(crate) fn rebuild_collection_summaries(&mut self) {
        self.ingest.collection_summaries.clear();
        for item in self.ingest.stored.values() {
            for collection in &item.collections {
                let entry = self
                    .ingest
                    .collection_summaries
                    .entry(*collection as usize)
                    .or_default();
                entry.0 += 1;
                entry.1 = Some(
                    entry
                        .1
                        .map_or(item.id as usize, |old| old.max(item.id as usize)),
                );
            }
        }
    }

    pub fn media_name(&self, id: usize) -> String {
        self.ingest
            .stored
            .get(&id)
            .map(|m| format!("danbooru_{}.{}", m.post.id, m.post.file_ext))
            .unwrap_or_else(|| format!("sample_{:03}.png", id + 1))
    }

    pub fn paint_media(&mut self, ui: &egui::Ui, rect: egui::Rect, id: usize) {
        if !ui.is_rect_visible(rect) {
            return;
        }
        if self.demo || self.is_capture() {
            sample_art(ui, rect, id);
            return;
        }
        if !self.ingest.textures.contains_key(&id) {
            if let Some(item) = self.ingest.stored.get(&id) {
                self.ingest
                    .thumbnails
                    .request(ui.ctx(), id, item.thumbnail.clone());
            }
        }
        ui.painter().rect_filled(rect, 7.0, BG);
        if let Some(texture) = self.ingest.textures.get(&id) {
            self.ingest.thumbnails.touch(id);
            let fit = egui::Rect::from_center_size(
                rect.center(),
                texture.size_vec2()
                    * (rect.width() / texture.size_vec2().x)
                        .min(rect.height() / texture.size_vec2().y),
            );
            ui.painter().image(
                texture.id(),
                fit,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            icon(
                ui,
                egui::Rect::from_center_size(rect.center(), vec2(30.0, 30.0)),
                "image",
                MUTED,
            );
        }
    }

    pub(crate) fn review_texture(
        &mut self,
        ctx: &egui::Context,
        id: usize,
    ) -> Option<egui::TextureHandle> {
        if self.demo || self.is_capture() || self.ingest.detail_requested != Some(id) {
            return None;
        }
        if let Some((pending_id, receiver)) = &self.ingest.detail_pending {
            match receiver.try_recv() {
                Ok(Ok(decoded)) => {
                    self.ingest.detail_texture = Some((
                        *pending_id,
                        ctx.load_texture(
                            format!("original-{pending_id}"),
                            decoded,
                            egui::TextureOptions::LINEAR,
                        ),
                    ));
                    self.ingest.detail_pending = None;
                }
                Ok(Err(error)) => {
                    self.ingest.detail_error = Some((*pending_id, error));
                    self.ingest.detail_pending = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.ingest.detail_error = Some((*pending_id, "原图查看任务已中断。".into()));
                    self.ingest.detail_pending = None;
                }
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
        if self.ingest.detail_pending.is_none()
            && self
                .ingest
                .detail_texture
                .as_ref()
                .is_none_or(|(current, _)| *current != id)
            && self
                .ingest
                .detail_error
                .as_ref()
                .is_none_or(|(failed, _)| *failed != id)
        {
            if let Some(media) = self.ingest.stored.get(&id) {
                let path = media.path.clone();
                let repaint = ctx.clone();
                let (sender, receiver) = mpsc::channel();
                std::thread::spawn(move || {
                    let result = (|| {
                        let mut reader = image::ImageReader::open(path)
                            .map_err(|_| "无法读取原图。".to_owned())?;
                        let mut limits = image::Limits::default();
                        limits.max_alloc = Some(256 * 1024 * 1024);
                        reader.limits(limits);
                        let decoded = reader
                            .decode()
                            .map_err(|_| "无法解码原图，请检查文件。".to_owned())?
                            .to_rgba8();
                        Ok(egui::ColorImage::from_rgba_unmultiplied(
                            [decoded.width() as usize, decoded.height() as usize],
                            decoded.as_raw(),
                        ))
                    })();
                    let _ = sender.send(result);
                    repaint.request_repaint();
                });
                self.ingest.detail_texture = None;
                self.ingest.detail_pending = Some((id, receiver));
            }
        }
        self.ingest
            .detail_texture
            .as_ref()
            .filter(|(current, _)| *current == id)
            .map(|(_, texture)| texture.clone())
    }

    pub fn ingest_active(&self) -> bool {
        self.ingest.pending.is_some()
    }

    pub fn start_ingest(&mut self, ctx: &egui::Context, resume: Option<i64>) -> Result<(), String> {
        if self.demo || self.is_capture() {
            return Err("示例模式不下载图片。".into());
        }
        if self.ingest_active() {
            return Err("已有下载任务，请等待或取消。".into());
        }
        if self.image_editor.active() {
            return Err("正在生成修复预览，请等待或取消后再下载。".into());
        }
        if self.workflows.metadata_active {
            return Err("正在更新标签关系，请完成后再开始下载。".into());
        }
        if self.workflow_blocks_ingest() {
            return Err("正在处理图片，请等待完成或暂停处理任务后再下载。".into());
        }
        let path = self.catalog.path.clone().ok_or("本地数据库未打开。")?;
        let credentials = self
            .saved_danbooru_credentials()
            .map_err(|e| e.to_string())?;
        let mut repo =
            bdm_infrastructure::catalog::SqliteCatalog::open(&path).map_err(|e| e.to_string())?;
        let jobs = if let Some(id) = resume {
            vec![repo
                .list_jobs()
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|j| j.id == id && j.state != "completed")
                .ok_or("任务已变化。")?]
        } else {
            let ids = if let Some(id) = self.context {
                BTreeSet::from([id])
            } else {
                self.selected.clone()
            };
            if ids.is_empty() {
                return Err("请先选择画师或角色，或进入其图库。".into());
            }
            let root = if self.prefs.root.is_empty() {
                crate::paths::library_directory()
            } else {
                PathBuf::from(&self.prefs.root)
            };
            self.library
                .collections(self.prefs.page)
                .iter()
                .filter(|c| ids.contains(&c.id))
                .map(|c| {
                    repo.new_job_with_transparency(
                        c.id as i64,
                        c.tag.clone(),
                        root.clone(),
                        self.prefs.minimum_short_side,
                        self.prefs.flatten_transparency,
                    )
                    .map_err(|e| e.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name("image-ingestion".into())
            .spawn(move || {
                let provider = bdm_infrastructure::danbooru::DanbooruProvider::new(credentials);
                for mut job in jobs {
                    let result = match &provider {
                        Ok(provider) => bdm_application::ingest::run(
                            provider,
                            &mut repo,
                            &mut job,
                            &signal,
                            |j| {
                                let _ = sender.send(j.clone());
                                repaint.request_repaint();
                            },
                        ),
                        Err(error) => Err(bdm_ports::IngestError::Catalog(*error)),
                    };
                    if let Err(error) = result {
                        job.state = if error == bdm_ports::IngestError::Cancelled {
                            "cancelled"
                        } else {
                            "failed"
                        }
                        .into();
                        if job.error.is_none() {
                            job.error = Some(error.to_string());
                        }
                        let _ = bdm_ports::IngestRepository::save_job(&mut repo, &job);
                        let _ = sender.send(job);
                        break;
                    }
                }
                repaint.request_repaint();
            })
            .map_err(|_| "无法启动下载线程。")?;
        self.ingest.pending = Some(receiver);
        self.workflows.auto_analysis_enabled = true;
        self.ingest.cancel = Some(cancel);
        self.ingest.error = None;
        self.log("INFO", "图片下载任务已启动。");
        Ok(())
    }

    pub fn poll_ingest(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.ingest.pending else {
            return;
        };
        let mut changed = false;
        let mut done = false;
        loop {
            match receiver.try_recv() {
                Ok(job) => {
                    if let Some(old) = self.ingest.jobs.iter_mut().find(|j| j.id == job.id) {
                        *old = job;
                    } else {
                        self.ingest.jobs.push(job);
                    }
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    done = true;
                    break;
                }
            }
        }
        if done {
            self.ingest.pending = None;
            self.ingest.cancel = None;
        }
        self.ingest.refresh_pending |= changed;
        let refresh = done
            || (self.ingest.refresh_pending
                && self
                    .ingest
                    .last_refresh
                    .is_none_or(|last| last.elapsed() >= std::time::Duration::from_millis(350)));
        if refresh {
            self.ingest.refresh_pending = false;
            self.ingest.last_refresh = Some(std::time::Instant::now());
            if let Err(error) = self.reload_media() {
                self.ingest.error = Some(error);
            }
        }
        if refresh {
            self.start_background_analysis(ctx);
        }
        if self.ingest_active() {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
    }

    pub fn download_controls(&mut self, ui: &mut egui::Ui) {
        if self.demo || self.is_capture() {
            return;
        }
        let enabled = !self.demo
            && !self.is_capture()
            && !self.ingest_active()
            && !self.workflows.metadata_active
            && !self.workflow_blocks_ingest()
            && (self.context.is_some() || !self.selected.is_empty());
        if ui
            .add_enabled_ui(enabled, |ui| icon_button(ui, "download", "下载图片", false))
            .inner
            .on_disabled_hover_text("选择画师／角色卡片，或进入单个收藏的图库。")
            .clicked()
        {
            match self.start_ingest(ui.ctx(), None) {
                Ok(()) => self.notify(ui.ctx(), "下载任务已启动，可在总览查看进度。"),
                Err(error) => {
                    self.notify(ui.ctx(), &error);
                    self.ingest.error = Some(error);
                }
            }
        }
    }

    pub fn ingestion_status(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = &self.ingest.error {
            ui.add(egui::Label::new(RichText::new(error).size(12.0).color(DANGER)).wrap());
        }
        if self.ingest.jobs.is_empty() {
            return;
        }
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            section(ui, "图片下载", "");
            ui.add_space(10.0);
            for (index, job) in self
                .ingest
                .jobs
                .clone()
                .into_iter()
                .rev()
                .take(6)
                .enumerate()
            {
                if index > 0 {
                    ui.separator();
                    ui.add_space(6.0);
                }
                let (state, color) = task_state(&job.state);
                let active = self.ingest_active();
                let action = if active && job.state == "running" {
                    Some("暂停")
                } else if !active && job.state != "completed" {
                    Some("继续")
                } else {
                    None
                };
                let clicked = task_row(
                    ui,
                    TaskRow {
                        kind: "image",
                        title: &job.tag,
                        subtitle: &format!("短边 ≥ {} px", job.minimum_short_side),
                        state,
                        color,
                        action,
                        enabled: !self.demo && !self.is_capture(),
                        error: job.error.as_deref(),
                    },
                    |ui| {
                        for (label, value, color) in [
                            ("检查", job.progress.examined, MUTED),
                            ("入库", job.progress.imported, ACCENT),
                            ("复用", job.progress.duplicates, MUTED),
                            ("跳过", job.progress.skipped, MUTED),
                            (
                                "失败",
                                job.progress.failed,
                                if job.progress.failed > 0 {
                                    DANGER
                                } else {
                                    MUTED
                                },
                            ),
                        ] {
                            task_metric(ui, label, value, color);
                        }
                    },
                );
                if clicked {
                    if active {
                        if let Some(cancel) = &self.ingest.cancel {
                            cancel.store(true, Ordering::Relaxed);
                        }
                    } else if let Err(error) = self.start_ingest(ui.ctx(), Some(job.id)) {
                        self.ingest.error = Some(error);
                    }
                }
            }
        });
        ui.add_space(12.0);
    }
}
