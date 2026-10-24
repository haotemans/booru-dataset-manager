use crate::{app::BdmApp, theme::*, widgets::*};
use bdm_domain::{
    dedup::DuplicateCandidate,
    tags::{CharacterRelation, TagRules},
};
use bdm_infrastructure::workflows::{TextArtifact, WorkflowJob};
use eframe::egui::{self, RichText};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
};

enum Event {
    Job(Box<WorkflowJob>),
    Rules(TagRules),
    Finished(Result<(), String>),
}

pub struct ImageUploadConfirmation {
    pub profile: crate::model::ApiProfile,
    pub ids: BTreeSet<i64>,
    pub resume: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdm_ports::{CollectionRepository, IngestRepository, MediaProvider};
    use md5::{Digest, Md5};

    struct PipelineFixture {
        path: PathBuf,
        app: BdmApp,
    }
    impl PipelineFixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "bdm-pipeline-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let mut app =
                BdmApp::from_preferences(crate::model::Preferences::default(), false, None);
            app.catalog.path = Some(path.join("catalog.sqlite3"));
            app.prefs.model_path = path.join("missing-model").display().to_string();
            let mut repo = app.workflow_repo().unwrap();
            let collection = repo
                .add(
                    bdm_domain::catalog::CollectionKind::Character,
                    &bdm_domain::catalog::SourceTag {
                        id: 1,
                        name: "character".into(),
                        category: 4,
                        post_count: 2,
                    },
                )
                .unwrap()
                .record
                .id;
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                800,
                800,
                image::Rgb([30, 60, 90]),
            ))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
            let bytes = bytes.into_inner();
            let post = bdm_domain::ingest::SourcePost {
                id: 10,
                md5: format!("{:x}", Md5::digest(&bytes)),
                file_url: Some("https://cdn.donmai.us/image.png".into()),
                file_ext: "png".into(),
                file_size: bytes.len() as u64,
                image_width: 800,
                image_height: 800,
                tag_string: "character scenery".into(),
                tag_string_artist: String::new(),
                tag_string_character: "character".into(),
                tag_string_general: "scenery".into(),
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
                ) -> Result<Vec<bdm_domain::ingest::SourcePost>, bdm_ports::IngestError>
                {
                    Ok(Vec::new())
                }
                fn download(
                    &self,
                    _: &bdm_domain::ingest::SourcePost,
                    path: &std::path::Path,
                    _: &AtomicBool,
                ) -> Result<(), bdm_ports::IngestError> {
                    std::fs::write(path, &self.0).map_err(|_| bdm_ports::IngestError::Storage)
                }
            }
            repo.import(
                &post,
                &path.join("library"),
                collection,
                800,
                &AtomicBool::new(false),
                &Provider(bytes),
            )
            .unwrap();
            app.reload_media().unwrap();
            Self { path, app }
        }
        fn finish(&mut self, ctx: &egui::Context) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while self.app.workflow_active() && std::time::Instant::now() < deadline {
                self.app.poll_workflows(ctx);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(!self.app.workflow_active());
        }
    }
    impl Drop for PipelineFixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }

    #[test]
    fn imported_local_tags_are_not_review_optimization_results_and_reading_starts_no_jobs() {
        let mut f = PipelineFixture::new();
        f.app.reload_workflow_data().unwrap();
        let media = f.app.library.media[0].clone();
        let raw = f.app.ingest.stored[&media.id].post.tag_string.clone();
        let before = f.app.workflow_repo().unwrap().list_artifacts().unwrap();
        assert!(before
            .iter()
            .any(|a| a.provenance == "local-normalization-v1"));
        assert!(!media.optimized);
        assert!(!media.caption);
        for tab in 0..3 {
            assert_eq!(
                f.app.review_text(&media, tab),
                if tab == 0 { raw.clone() } else { String::new() }
            );
            assert!(f.app.review_output(media.id, tab).is_none());
        }
        assert!(f
            .app
            .workflow_repo()
            .unwrap()
            .list_workflows()
            .unwrap()
            .is_empty());
        let after = f.app.workflow_repo().unwrap().list_artifacts().unwrap();
        let snapshot = |artifacts: &[TextArtifact]| {
            artifacts
                .iter()
                .map(|a| {
                    (
                        a.media_id,
                        a.kind.clone(),
                        a.content.clone(),
                        a.state.clone(),
                        a.reviewed,
                        a.provenance.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(snapshot(&before), snapshot(&after));
        assert!(!f.app.workflow_active());
    }

    #[test]
    fn review_generated_targets_are_independent_and_pending_review_outputs_stay_visible() {
        for kind in ["tags", "caption"] {
            let mut f = PipelineFixture::new();
            let stored = f.app.ingest.stored.values().next().unwrap().clone();
            let raw = stored.post.tag_string.clone();
            let content = if kind == "tags" {
                "character, scenery"
            } else {
                "A landscape with mountains.\nClear sky."
            };
            f.app
                .workflow_repo()
                .unwrap()
                .write_artifact(
                    &stored,
                    kind,
                    content,
                    "synthetic-test:model:job-1:bdm-text-v1",
                    "needs_review",
                )
                .unwrap();
            f.app.reload_workflow_data().unwrap();
            let media = f.app.library.media[0].clone();
            let tab = if kind == "tags" { 1 } else { 2 };
            assert_eq!(f.app.review_text(&media, 0), raw);
            assert_eq!(f.app.review_text(&media, tab), content);
            assert_eq!(f.app.review_text(&media, 3 - tab), "");
            assert!(!f.app.review_output(media.id, tab).unwrap().reviewed);
            f.app
                .workflow_repo()
                .unwrap()
                .review_artifact(stored.id, kind)
                .unwrap();
            f.app.reload_workflow_data().unwrap();
            assert!(f.app.review_output(media.id, tab).unwrap().reviewed);
            assert_eq!(f.app.review_text(&media, tab), content);
            assert_eq!(f.app.ingest.stored[&media.id].post.tag_string, raw);
            assert!(f
                .app
                .workflow_repo()
                .unwrap()
                .list_workflows()
                .unwrap()
                .is_empty());
        }
    }

    #[test]
    fn pipeline_analyzes_during_download_without_text_jobs_or_missing_model_loop() {
        let mut f = PipelineFixture::new();
        let ctx = egui::Context::default();
        assert!(!f.app.workflows.auto_analysis_enabled); // Restart requires explicit action.
        let (sender, receiver) = mpsc::channel();
        f.app.pending_for_test(receiver);
        assert!(f.app.ingest_active());
        f.app.workflows.auto_analysis_enabled = true;
        f.app.start_background_analysis(&ctx);
        assert!(f.app.workflow_active());
        assert!(!f.app.workflow_blocks_ingest());
        f.finish(&ctx);
        assert_eq!(f.app.workflows.jobs.len(), 1);
        assert_eq!(f.app.workflows.jobs[0].kind, "dedup-auto");
        assert_eq!(f.app.workflows.jobs[0].state, "needs_model");
        f.app.start_background_analysis(&ctx);
        assert!(!f.app.workflow_active());
        assert_eq!(f.app.workflows.jobs.len(), 1);
        assert!(f
            .app
            .workflows
            .artifacts
            .keys()
            .all(|(_, kind)| kind == "tags"));
        assert!(f
            .app
            .workflows
            .artifacts
            .values()
            .all(|a| a.provenance == "local-normalization-v1"));
        drop(sender);
        f.app.poll_ingest(&ctx);
        assert!(!f.app.workflow_active());
        assert!(!f.app.workflows.metadata_active);
    }

    #[test]
    fn applied_relationships_stay_off_collection_page_and_settings_open_only_on_request() {
        for size in [egui::vec2(1050.0, 720.0), egui::vec2(1440.0, 960.0)] {
            let ctx = egui::Context::default();
            crate::theme::setup(&ctx);
            let mut app = BdmApp::from_preferences(
                crate::model::Preferences::default(),
                false,
                Some(PathBuf::from("synthetic-capture")),
            );
            app.workflows.rules.characters = (0..80)
                .map(|index| CharacterRelation {
                    base: format!("base_{index}"),
                    variant: format!("variant_{index}"),
                    verified: true,
                    exclusive_group: String::new(),
                    evidence: if index % 2 == 0 {
                        bdm_domain::tags::DANBOORU_CHARACTER_IMPLICATION.into()
                    } else {
                        "human-confirmed".into()
                    },
                })
                .collect();
            let snapshot = app.workflows.rules.clone();
            app.workflows.relationship_summary = Some("关系与本地标签已更新。".into());
            let height = std::cell::Cell::new(0.0);
            ctx.style_mut(|style| style.animation_time = 0.0);
            let frame = |app: &mut BdmApp, surface: usize, events: Vec<egui::Event>| {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let start = ui.cursor().top();
                            if surface == 1 {
                                app.settings_page(ui);
                            } else {
                                app.character_relationships(ui);
                            }
                            height.set(ui.cursor().top() - start);
                        });
                        if surface == 2 {
                            app.show_dialog(ctx);
                        }
                    },
                );
                output
                    .shapes
                    .into_iter()
                    .filter_map(|shape| match shape.shape {
                        egui::epaint::Shape::Text(text) => {
                            Some((text.galley.text().to_owned(), text.pos))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            for _ in 0..3 {
                frame(&mut app, 0, Vec::new());
            }
            let text = frame(&mut app, 0, Vec::new());
            assert!(
                height.get() < 1.0,
                "completed automatic work must use no collection space"
            );
            assert!(
                text.is_empty(),
                "completed work must show no status, counts or relation controls: {text:?}"
            );
            for _ in 0..3 {
                frame(&mut app, 1, Vec::new());
            }
            let text = frame(&mut app, 1, Vec::new());
            let pos =
                text.iter().find(|(s, _)| s == "角色关系设置").unwrap().1 + egui::vec2(30.0, 8.0);
            for pressed in [true, false] {
                frame(
                    &mut app,
                    1,
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
            assert!(matches!(
                app.dialog,
                Some(crate::app::Dialog::CharacterRelations)
            ));
            for _ in 0..3 {
                frame(&mut app, 2, Vec::new());
            }
            let text = frame(&mut app, 2, Vec::new());
            assert!(text.iter().any(|(s, _)| s.contains("base_0")));
            assert!(text.iter().any(|(s, _)| s == "添加关系"));
            assert!(
                !text.iter().any(|(s, _)| s == "确认关系"),
                "applied relations must not ask for a second review"
            );
            assert_eq!(app.workflows.rules, snapshot);
            assert!(!app.workflow_active());
            frame(
                &mut app,
                2,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
            );
            assert!(app.dialog.is_none());
            let text = frame(&mut app, 0, Vec::new());
            assert!(text.is_empty());
            assert!(height.get() < 1.0);
            app.workflows.rules.characters.push(CharacterRelation {
                base: "pending_base".into(),
                variant: "pending_variant".into(),
                verified: false,
                exclusive_group: String::new(),
                evidence: "ambiguous".into(),
            });
            let text = frame(&mut app, 0, Vec::new());
            assert!(text.iter().any(|(s, _)| s.contains("pending_base")));
            assert!(text.iter().any(|(s, _)| s == "确认关系"));
            assert!(!text.iter().any(|(s, _)| s.contains("base_0")));
            assert!(!text
                .iter()
                .any(|(s, _)| s == "处理状态" || s == "自动处理" || s.contains("管理已确认")));
        }
    }
    #[test]
    fn cached_source_relationships_process_on_character_page_without_network_and_survive_reopen() {
        let mut f = PipelineFixture::new();
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let mut rules = f.app.workflows.rules.clone();
        rules
            .categories
            .extend([("base".into(), 4), ("character".into(), 4)]);
        rules.characters.push(CharacterRelation {
            base: "base".into(),
            variant: "character".into(),
            verified: false,
            exclusive_group: String::new(),
            evidence: "Danbooru active implication; semantic hierarchy awaits confirmation".into(),
        });
        f.app
            .workflow_repo()
            .unwrap()
            .save_tag_rules(&rules)
            .unwrap();
        f.app.workflows.rules = rules;
        let before = f.app.workflows.artifacts.clone();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1050.0, 720.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| f.app.character_relationships(ui));
            },
        );
        assert!(f.app.workflow_active());
        f.finish(&ctx);
        assert!(f.app.workflows.rules.characters[0].source_verified());
        assert_eq!(f.app.workflows.rules.characters[0].exclusive_group, "");
        assert!(f
            .app
            .workflows
            .relationship_summary
            .as_ref()
            .unwrap()
            .contains("已更新"));
        assert!(f
            .app
            .workflow_repo()
            .unwrap()
            .tag_rules()
            .unwrap()
            .characters[0]
            .source_verified());
        assert_eq!(
            f.app.workflows.artifacts.keys().collect::<Vec<_>>(),
            before.keys().collect::<Vec<_>>()
        );
        f.app.process_discovered_relationships(&ctx);
        assert!(
            !f.app.workflow_active(),
            "idempotent processing must not launch another worker"
        );
    }

    #[test]
    fn explicit_pause_prevents_new_imports_from_restarting_analysis() {
        let mut f = PipelineFixture::new();
        let ctx = egui::Context::default();
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        f.app.workflows.pending = Some(receiver);
        f.app.workflows.cancel = Some(cancel.clone());
        f.app.workflows.active_kind = Some("dedup-auto".into());
        f.app.workflows.auto_analysis_enabled = true;
        f.app.pause_processing();
        assert!(cancel.load(Ordering::Relaxed));
        sender.send(Event::Finished(Ok(()))).unwrap();
        f.app.poll_workflows(&ctx);
        f.app.start_background_analysis(&ctx);
        assert!(!f.app.workflow_active());
        assert!(f
            .app
            .workflow_repo()
            .unwrap()
            .list_workflows()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn active_processing_blocks_download_before_creating_jobs_or_reading_credentials() {
        let mut app = BdmApp::from_preferences(crate::model::Preferences::default(), false, None);
        let (_sender, receiver) = mpsc::channel();
        app.workflows.pending = Some(receiver);
        let error = app
            .start_ingest(&egui::Context::default(), None)
            .unwrap_err();
        assert!(error.contains("正在处理"));
        assert!(app.ingest.jobs.is_empty());
    }
    #[test]
    fn similarity_candidates_are_sorted_scoped_and_paginated_without_dispatch() {
        let mut app = BdmApp::from_preferences(crate::model::Preferences::default(), true, None);
        app.action_snapshot = (0..24).collect();
        app.workflows.candidates = (1..23)
            .map(|id| DuplicateCandidate {
                first: 0,
                second: id,
                phash_distance: Some(12),
                cosine: Some(0.90 + id as f32 * 0.003),
                model_version: Some("fixture".into()),
                state: "pending".into(),
            })
            .collect();
        assert_eq!(app.similarity_candidates().len(), 22);
        assert_eq!(app.similarity_candidates()[0].second, 22);
        app.workflows.candidate_page = 4;
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        app.dialog = Some(crate::app::Dialog::Action("去重".into()));
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1050.0, 720.0),
                )),
                ..Default::default()
            },
            |ctx| app.shell(ctx),
        );
        assert_eq!(app.workflows.candidate_page, 4);
        app.selected_media = BTreeSet::from([0, 22]);
        app.action_selected = true;
        assert_eq!(app.similarity_candidates().len(), 1);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1050.0, 720.0),
                )),
                ..Default::default()
            },
            |ctx| app.shell(ctx),
        );
        assert_eq!(app.workflows.candidate_page, 0);
        assert!(app.workflows.jobs.is_empty());
    }

    #[test]
    fn invalid_configuration_never_creates_a_resumable_job() {
        let path = std::env::temp_dir().join(format!(
            "bdm-workflow-config-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut app = BdmApp::from_preferences(crate::model::Preferences::default(), false, None);
        app.catalog.path = Some(path.clone());
        let ctx = egui::Context::default();
        assert!(app.start_workflow(&ctx, "optimize", vec![1], None).is_err());
        assert!(app.start_workflow(&ctx, "export", vec![1], None).is_err());
        assert!(app.start_workflow(&ctx, "dedup", vec![], None).is_err());
        assert!(app
            .workflow_repo()
            .unwrap()
            .list_workflows()
            .unwrap()
            .is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn completed_and_superseded_tasks_leave_dashboard_clean_and_history_stays_available() {
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let mut app = BdmApp::from_preferences(crate::model::Preferences::default(), true, None);
        app.workflows.jobs = vec![
            WorkflowJob {
                id: 1,
                kind: "dedup-auto".into(),
                ids: vec![1, 2],
                cursor: 1,
                state: "interrupted".into(),
                ..Default::default()
            },
            WorkflowJob {
                id: 2,
                kind: "dedup-auto".into(),
                ids: vec![2],
                cursor: 1,
                state: "completed".into(),
                ..Default::default()
            },
        ];
        assert!(app.actionable_workflows().is_empty());
        let height = std::cell::Cell::new(0.0);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let before = ui.cursor().top();
                app.workflow_status(ui);
                height.set(ui.cursor().top() - before);
            });
        });
        assert!(
            height.get() < 1.0,
            "finished work must not leave an empty task card"
        );
        assert_eq!(app.workflows.jobs.len(), 2);
        app.workflows.jobs[0].ids.push(3);
        assert_eq!(app.actionable_workflows().len(), 1);
        app.workflows.candidates = vec![DuplicateCandidate {
            first: 1,
            second: 2,
            phash_distance: Some(12),
            cosine: Some(0.96),
            model_version: Some("fixture".into()),
            state: "pending".into(),
        }];
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.workflow_status(ui));
        });
        fn texts(shape: &egui::Shape, result: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => result.push(text.galley.text().into()),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        texts(shape, result)
                    }
                }
                _ => {}
            }
        }
        let mut text = Vec::new();
        for shape in output.shapes {
            texts(&shape.shape, &mut text);
        }
        assert!(text.iter().any(|text| text == "看图审核"));
        assert!(!text.iter().any(|text| text.contains("旧任务")
            || text.contains("历史任务")
            || text.contains("待审核候选")
            || text.contains("重新筛查")));
    }

    #[test]
    fn scan_completion_opens_visual_pairs_but_never_interrupts_an_existing_review() {
        let mut f = PipelineFixture::new();
        let ctx = egui::Context::default();
        let (sender, receiver) = mpsc::channel();
        f.app.workflows.pending = Some(receiver);
        f.app.dialog = Some(crate::app::Dialog::Action("去重".into()));
        sender.send(Event::Finished(Ok(()))).unwrap();
        f.app.poll_workflows(&ctx);
        assert!(f.app.dialog.is_none());
        // Inject synthetic media/candidates in demo mode to test the completion handoff only.
        let mut app = BdmApp::from_preferences(crate::model::Preferences::default(), true, None);
        app.action_snapshot = vec![1, 2];
        app.workflows.candidates = vec![DuplicateCandidate {
            first: 1,
            second: 2,
            phash_distance: Some(12),
            cosine: Some(0.96),
            model_version: Some("fixture".into()),
            state: "pending".into(),
        }];
        for existing in [
            crate::app::Dialog::Action("去重".into()),
            crate::app::Dialog::Media(1),
        ] {
            let (sender, receiver) = mpsc::channel();
            app.workflows.pending = Some(receiver);
            app.dialog = Some(existing);
            sender.send(Event::Finished(Ok(()))).unwrap();
            app.poll_workflows(&ctx);
            assert!(matches!(
                app.dialog,
                Some(crate::app::Dialog::Similarity | crate::app::Dialog::Media(1))
            ));
        }
    }
}
#[derive(Default)]
pub struct WorkflowSettings {
    pub jobs: Vec<WorkflowJob>,
    pub artifacts: BTreeMap<(usize, String), TextArtifact>,
    pub rules: TagRules,
    pub candidates: Vec<DuplicateCandidate>,
    pub profile: usize,
    pub error: Option<String>,
    pub relationship_summary: Option<String>,
    pub base: String,
    pub variant: String,
    pub export_root: String,
    pub candidate_page: usize,
    auto_analysis_waiting: bool,
    pub(crate) auto_analysis_enabled: bool,
    pub metadata_active: bool,
    active_kind: Option<String>,
    pending: Option<Receiver<Event>>,
    cancel: Option<Arc<AtomicBool>>,
}

impl BdmApp {
    pub fn workflow_active(&self) -> bool {
        self.workflows.pending.is_some() || self.similarity_review.is_saving()
    }
    pub fn workflow_blocks_ingest(&self) -> bool {
        self.workflow_active()
            && !self
                .workflows
                .active_kind
                .as_deref()
                .is_some_and(|kind| kind.starts_with("dedup"))
    }
    fn workflow_repo(&self) -> Result<bdm_infrastructure::catalog::SqliteCatalog, String> {
        bdm_infrastructure::catalog::SqliteCatalog::open(
            self.catalog.path.as_deref().ok_or("本地数据库未打开。")?,
        )
        .map_err(|e| e.to_string())
    }
    pub fn model_directory(&self) -> PathBuf {
        if !self.prefs.model_path.is_empty() {
            return PathBuf::from(&self.prefs.model_path);
        }
        crate::paths::project_root().join("models/dinov3")
    }
    pub fn reload_workflow_data(&mut self) -> Result<(), String> {
        let repo = self.workflow_repo()?;
        self.workflows.jobs = repo.list_workflows().map_err(|e| e.to_string())?;
        let rules = repo.tag_rules().map_err(|e| e.to_string())?;
        if self.workflows.rules != rules {
            self.tags.invalidate();
            self.workflows.rules = rules;
        }
        self.workflows.candidates = repo.list_candidates().map_err(|e| e.to_string())?;
        self.workflows.artifacts = repo
            .list_artifacts()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|a| ((a.media_id as usize, a.kind.clone()), a))
            .collect();
        for media in &mut self.library.media {
            media.optimized = self
                .workflows
                .artifacts
                .get(&(media.id, "tags".into()))
                .is_some_and(|a| a.state == "complete" && a.provenance != "local-normalization-v1");
            media.caption = self
                .workflows
                .artifacts
                .get(&(media.id, "caption".into()))
                .is_some_and(|a| a.state == "complete");
        }
        Ok(())
    }
    pub fn start_background_analysis(&mut self, ctx: &egui::Context) {
        if !self.workflows.auto_analysis_enabled {
            return;
        }
        self.workflows.auto_analysis_waiting = true;
        if self.demo || self.is_capture() || self.workflow_active() || self.ingest.stored.is_empty()
        {
            return;
        }
        let ids = match self.workflow_repo().and_then(|repo| {
            repo.pending_analysis_batch(
                64,
                self.model_directory().join("model.safetensors").is_file(),
            )
            .map_err(|e| e.to_string())
        }) {
            Ok(ids) => ids,
            Err(error) => {
                self.workflows.auto_analysis_enabled = false;
                self.workflows.error = Some(error);
                return;
            }
        };
        if ids.is_empty() {
            self.workflows.auto_analysis_waiting = false;
            return;
        }
        self.workflows.auto_analysis_waiting = false;
        if let Err(e) = self.start_workflow(ctx, "dedup-auto", ids, None) {
            self.workflows.auto_analysis_enabled = false;
            self.workflows.error = Some(e);
        }
    }
    pub fn action_ids(&self) -> Vec<i64> {
        self.action_snapshot
            .iter()
            .filter(|id| !self.action_selected || self.selected_media.contains(id))
            .map(|id| *id as i64)
            .collect()
    }

    pub fn start_workflow(
        &mut self,
        ctx: &egui::Context,
        kind: &str,
        ids: Vec<i64>,
        resume: Option<i64>,
    ) -> Result<(), String> {
        self.start_workflow_with_approval(ctx, kind, ids, resume, None)
    }

    fn start_workflow_with_approval(
        &mut self,
        ctx: &egui::Context,
        kind: &str,
        ids: Vec<i64>,
        resume: Option<i64>,
        approval: Option<bdm_infrastructure::ai::UploadApproval>,
    ) -> Result<(), String> {
        if self.demo || self.is_capture() {
            return Err("示例模式不执行后台任务。".into());
        }
        if self.workflow_active() {
            return Err("已有处理任务，可暂停后继续。".into());
        }
        if self.image_editor.active() {
            return Err("正在生成修复预览，请等待或取消后再启动处理。".into());
        }
        let mut repo = self.workflow_repo()?;
        let mut job = if let Some(id) = resume {
            repo.list_workflows()
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|j| j.id == id && j.state != "completed")
                .ok_or("任务不存在。")?
        } else {
            if ids.is_empty() || !matches!(kind, "dedup" | "dedup-auto" | "optimize" | "export") {
                return Err("请选择图片和有效的处理任务。".into());
            }
            WorkflowJob {
                kind: kind.into(),
                ids,
                state: "queued".into(),
                fallback: 8,
                ..Default::default()
            }
        };
        if self.ingest_active() && !job.kind.starts_with("dedup") {
            return Err(
                "正在下载图片，请在下载和去重结束后手动启动标签优化、描述生成或导出。".into(),
            );
        }
        if job.kind.starts_with("dedup") && job.state == "needs_model" {
            job.cursor = 0;
        }
        if job.kind.starts_with("dedup") && job.state == "deferred" {
            if job.analysis_scope.is_empty() {
                job.analysis_scope = job.ids.iter().copied().collect();
            }
            job.ids = std::mem::take(&mut job.deferred);
            job.cursor = 0;
            job.analysis_attempts.clear();
        }
        if job.kind == "export" && resume.is_none() {
            job.mode = if self.action_mode == 0 {
                "caption"
            } else {
                "tags"
            }
            .into();
            let destination = std::path::Path::new(self.workflows.export_root.trim());
            if !destination.is_absolute() {
                return Err("请输入导出目录的绝对路径。".into());
            }
            job.destination = destination.display().to_string();
        }
        if job.kind.starts_with("dedup") && resume.is_none() && self.prefs.similar_image_review {
            job.review_similarity = Some(self.prefs.review_similarity);
        }
        let profile = if job.kind == "optimize" {
            let profile = if resume.is_some() {
                self.prefs.profiles.iter().find(|p| p.id == job.profile_id)
            } else {
                self.prefs.profiles.get(self.workflows.profile)
            }
            .ok_or("请先在 API 配置页添加服务。")?;
            profile.validate().map_err(str::to_owned)?;
            job.profile_id = profile.id.clone();
            if resume.is_none() {
                job.mode = if self.action_mode == 0 {
                    "caption"
                } else {
                    "tags"
                }
                .into();
            }
            if profile.request.send_images && job.mode == "caption" && approval.is_none() {
                let ids = job
                    .ids
                    .iter()
                    .copied()
                    .filter(|id| !job.accepted.contains(id))
                    .collect();
                self.dialog = Some(crate::app::Dialog::ImageUpload(Box::new(
                    ImageUploadConfirmation {
                        profile: profile.clone(),
                        ids,
                        resume,
                    },
                )));
                return Ok(());
            }
            if let Some(approval) = &approval {
                let _ = approval.ids.len(); // Consent is scoped to this dispatch and is not persisted as a global setting.
                                            // Explicitly removed entries are excluded from this dispatch scope.
                job.ids
                    .retain(|id| job.accepted.contains(id) || approval.ids.contains(id));
                job.deferred.retain(|id| approval.ids.contains(id));
                job.recovery.retain(|id| approval.ids.contains(id));
                job.cursor = if resume.is_some() { job.ids.len() } else { 0 };
                if resume.is_some() {
                    job.recovery = job
                        .ids
                        .iter()
                        .copied()
                        .filter(|id| !job.accepted.contains(id))
                        .collect();
                }
            }
            let credentials = self
                .api_profiles
                .store
                .load(&profile.id)
                .map_err(|e| e.to_string())?;
            Some(bdm_infrastructure::ai::AiProfile {
                id: profile.id.clone(),
                endpoint: profile.endpoint.clone(),
                model: profile.model.clone(),
                group: profile.group.clone(),
                credentials,
                settings: profile.request.clone(),
                upload_approval: approval,
                artist_tag_format: self.prefs.artist_tag_format.key().into(),
                character_tag_format: self.prefs.character_tag_format.key().into(),
            })
        } else {
            None
        };
        if resume.is_none() {
            job.id = repo
                .new_workflow(&job.kind, job.ids.clone())
                .map_err(|e| e.to_string())?
                .id;
        }
        repo.save_workflow(&job).map_err(|e| e.to_string())?;
        let model = self.model_directory();
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        let active_kind = job.kind.clone();
        std::thread::Builder::new()
            .name("dataset-workflow".into())
            .spawn(move || {
                let report = |j: &WorkflowJob| {
                    let _ = sender.send(Event::Job(Box::new(j.clone())));
                    repaint.request_repaint();
                };
                let result = if job.kind.starts_with("dedup") {
                    bdm_infrastructure::visual::run(&mut repo, &mut job, &model, &signal, report)
                } else if job.kind == "export" {
                    let destination = PathBuf::from(&job.destination);
                    bdm_infrastructure::export::run(
                        &mut repo,
                        &mut job,
                        &destination,
                        &signal,
                        report,
                    )
                } else if let Some(profile) = profile {
                    bdm_infrastructure::ai::run(&mut repo, &mut job, &profile, &signal, report)
                } else {
                    Err(bdm_ports::IngestError::Storage)
                };
                let _ = sender.send(Event::Finished(result.map_err(|e| e.to_string())));
                repaint.request_repaint();
            })
            .map_err(|_| "无法启动处理线程。")?;
        self.workflows.pending = Some(receiver);
        self.workflows.active_kind = Some(active_kind.clone());
        if active_kind == "dedup-auto" {
            self.workflows.auto_analysis_enabled = true;
        }
        self.workflows.cancel = Some(cancel);
        self.workflows.error = None;
        Ok(())
    }

    pub fn image_upload_dialog(
        &mut self,
        ui: &mut egui::Ui,
        scope: &ImageUploadConfirmation,
    ) -> bool {
        ui.label(format!(
            "配置：{} · 模型：{}",
            scope.profile.name, scope.profile.model
        ));
        ui.label(
            RichText::new(&scope.profile.endpoint)
                .size(12.0)
                .color(MUTED),
        );
        ui.label("用途：生成自然语言描述。图片内容将发送到上述服务。");
        ui.label(format!(
            "共 {} 张，每批最多 {} 张；最长边 {} px，JPEG 质量 {}。",
            scope.ids.len(),
            scope.profile.request.batch_size,
            scope.profile.request.image_max_edge,
            scope.profile.request.jpeg_quality
        ));
        ui.label(
            RichText::new("费用及服务方保留策略未提供。可取消，或取消勾选以移除图片。")
                .size(12.0)
                .color(MUTED),
        );
        let mut selected = scope.ids.clone();
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .show(ui, |ui| {
                for &id in &scope.ids {
                    ui.horizontal(|ui| {
                        let mut checked = true;
                        if ui.checkbox(&mut checked, "").changed() && !checked {
                            selected.remove(&id);
                        }
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
                        self.paint_media(ui, rect, id as usize);
                        ui.label(self.media_name(id as usize));
                    });
                }
            });
        if selected != scope.ids {
            self.dialog = Some(crate::app::Dialog::ImageUpload(Box::new(
                ImageUploadConfirmation {
                    profile: scope.profile.clone(),
                    ids: selected.clone(),
                    resume: scope.resume,
                },
            )));
        }
        let mut close = false;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!selected.is_empty(), egui::Button::new("允许发送并开始"))
                .clicked()
            {
                let unchanged = self.prefs.profiles.iter().any(|p| p == &scope.profile);
                if !unchanged {
                    self.workflows.error = Some("API 配置已变化，请重新准备发送范围。".into());
                    close = true;
                    return;
                }
                let approval = bdm_infrastructure::ai::UploadApproval {
                    profile_id: scope.profile.id.clone(),
                    endpoint: scope.profile.endpoint.clone(),
                    model: scope.profile.model.clone(),
                    settings: scope.profile.request.clone(),
                    ids: selected.clone(),
                };
                self.workflows.profile = self
                    .prefs
                    .profiles
                    .iter()
                    .position(|p| p.id == scope.profile.id)
                    .unwrap();
                self.action_mode = 0;
                match self.start_workflow_with_approval(
                    ui.ctx(),
                    "optimize",
                    selected.iter().copied().collect(),
                    scope.resume,
                    Some(approval),
                ) {
                    Ok(()) => close = true,
                    Err(error) => self.workflows.error = Some(error),
                }
            }
            if ui.button("取消").clicked() {
                close = true;
            }
        });
        if let Some(error) = &self.workflows.error {
            ui.label(RichText::new(error).color(AMBER));
        }
        close
    }

    pub fn start_model_download(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.workflow_active() || self.demo || self.is_capture() {
            return Err("当前不能下载模型。".into());
        }
        let root = self.model_directory();
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name("dinov3-download".into())
            .spawn(move || {
                let result =
                    bdm_infrastructure::visual::download_model(&root).map_err(|e| e.to_string());
                let _ = sender.send(Event::Finished(result));
                repaint.request_repaint();
            })
            .map_err(|_| "无法启动模型下载。")?;
        self.workflows.pending = Some(receiver);
        self.workflows.active_kind = Some("model".into());
        self.workflows.error = None;
        Ok(())
    }

    pub fn poll_workflows(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.workflows.pending else {
            return;
        };
        let mut done = false;
        let mut changed = false;
        let mut finished = false;
        let mut stopped = false;
        loop {
            match receiver.try_recv() {
                Ok(Event::Job(job)) => {
                    let job = *job;
                    stopped |= matches!(job.state.as_str(), "paused" | "failed" | "interrupted");
                    if let Some(old) = self.workflows.jobs.iter_mut().find(|j| j.id == job.id) {
                        *old = job;
                    } else {
                        self.workflows.jobs.push(job);
                    }
                    changed = true;
                }
                Ok(Event::Rules(rules)) => {
                    self.tags.invalidate();
                    self.workflows.rules = rules;
                    changed = true;
                }
                Ok(Event::Finished(result)) => {
                    finished = true;
                    if let Err(e) = result {
                        stopped = true;
                        self.workflows.error = Some(e);
                        if self.workflows.metadata_active {
                            self.workflows.relationship_summary =
                                Some("处理未完成，请查看错误并重试。".into());
                        }
                    } else if self.workflows.metadata_active {
                        self.workflows.relationship_summary = Some("关系与本地标签已更新。".into());
                    }
                    done = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    done = true;
                    break;
                }
            }
        }
        if done {
            if (stopped || !finished)
                && self
                    .workflows
                    .active_kind
                    .as_deref()
                    .is_some_and(|kind| kind.starts_with("dedup"))
            {
                self.workflows.auto_analysis_enabled = false;
                self.workflows.auto_analysis_waiting = false;
            }
            let analysis_ready = self
                .workflows
                .active_kind
                .as_deref()
                .is_some_and(|kind| kind.starts_with("dedup") || kind == "model");
            self.workflows.auto_analysis_waiting |=
                analysis_ready && self.workflows.auto_analysis_enabled;
            self.workflows.pending = None;
            self.workflows.active_kind = None;
            self.workflows.cancel = None;
            self.workflows.metadata_active = false;
        }
        if changed || done {
            if let Err(e) = self.reload_media() {
                self.workflows.error = Some(e);
            }
        }
        if done && self.workflows.auto_analysis_waiting {
            self.start_background_analysis(ctx);
        }
        if done && matches!(self.dialog,Some(crate::app::Dialog::Action(ref kind)) if kind=="去重")
        {
            if let Some(pair) = self.similarity_candidates().first().cloned() {
                self.open_scoped_pair_review(&pair);
            } else if self.workflows.error.is_none() && !self.workflow_active() {
                self.dialog = None;
            }
        }
        if self.workflow_active() {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
    }

    pub fn workflow_status(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = &self.workflows.error {
            ui.add(egui::Label::new(RichText::new(error).color(AMBER)).wrap());
        }
        let jobs = self.actionable_workflows();
        let pairs = self.pending_similarity_pairs();
        if jobs.is_empty()
            && pairs.is_empty()
            && !self.workflows.metadata_active
            && !self.workflow_active()
        {
            return;
        }
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            for job in jobs {
                ui.horizontal_wrapped(|ui| {
                    let kind = if job.kind.starts_with("dedup") {
                        "layers"
                    } else if job.kind == "export" {
                        "download"
                    } else {
                        "spark"
                    };
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                    icon(ui, rect, kind, ACCENT);
                    if job.state == "running" {
                        ui.spinner();
                    }
                    ui.label(if job.kind.starts_with("dedup") {
                        "去重"
                    } else if job.kind == "export" {
                        "导出"
                    } else {
                        "文本优化"
                    });
                    let state = match job.state.as_str() {
                        "running" => "处理中",
                        "paused" | "interrupted" => "已暂停",
                        "needs_model" => "模型未就绪",
                        "queued" => "等待",
                        _ => "未完成",
                    };
                    ui.label(RichText::new(state).size(12.0).color(MUTED));
                    ui.add(
                        egui::ProgressBar::new(
                            job.cursor.min(job.ids.len()) as f32 / job.ids.len().max(1) as f32,
                        )
                        .desired_width(130.0)
                        .text(format!(
                            "{} / {}",
                            job.cursor,
                            job.ids.len()
                        )),
                    );
                    if !self.workflow_active()
                        && !self.demo
                        && !self.is_capture()
                        && icon_button(ui, "forward", "继续", false).clicked()
                    {
                        if let Err(error) =
                            self.start_workflow(ui.ctx(), &job.kind, Vec::new(), Some(job.id))
                        {
                            self.workflows.error = Some(error);
                        }
                    }
                    if let Some(error) = &job.error {
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                        icon(ui, rect, "logs", AMBER);
                        response.on_hover_text(error);
                    }
                });
            }
            if self.workflow_active() {
                ui.horizontal(|ui| {
                    if self.workflows.metadata_active {
                        ui.spinner();
                        ui.label("更新标签");
                    }
                    if icon_button(ui, "pause", "暂停处理", false).clicked() {
                        self.pause_processing();
                    }
                });
            }
            if !pairs.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("相似图").strong());
                    badge(ui, &pairs.len().to_string(), ACCENT);
                    if primary(ui, "看图审核").clicked() {
                        self.open_pair_review(&pairs[0]);
                    }
                });
                let width = ((ui.available_width() - 36.0) / 6.0).min(120.0);
                ui.horizontal(|ui| {
                    for pair in pairs.iter().take(3) {
                        ui.push_id((pair.first, pair.second), |ui| {
                            ui.horizontal(|ui| {
                                for id in [pair.first, pair.second] {
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(width, 84.0),
                                        egui::Sense::click(),
                                    );
                                    self.paint_media(ui, rect, id as usize);
                                    if response.on_hover_text("对比这两张图").clicked() {
                                        self.open_pair_review(pair);
                                    }
                                }
                            });
                        });
                    }
                });
            }
        });
        ui.add_space(12.0);
    }

    fn actionable_workflows(&self) -> Vec<WorkflowJob> {
        let valid: BTreeSet<_> = self
            .library
            .media
            .iter()
            .map(|media| media.id as i64)
            .collect();
        let mut jobs = self.workflows.jobs.clone();
        jobs.sort_by_key(|job| std::cmp::Reverse(job.id));
        let mut covered: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
        let mut kinds = BTreeSet::new();
        let mut visible = Vec::new();
        for job in jobs {
            if job.state == "completed" {
                covered
                    .entry(job.kind.clone())
                    .or_default()
                    .extend(job.ids.iter().copied());
                continue;
            }
            let remaining = if job.state == "needs_model" {
                &job.ids[..]
            } else if job.state == "deferred" {
                &job.deferred[..]
            } else {
                &job.ids[job.cursor.min(job.ids.len())..]
            };
            let relevant = remaining.iter().any(|id| {
                valid.contains(id) && !covered.get(&job.kind).is_some_and(|ids| ids.contains(id))
            });
            if (relevant
                || (self.workflow_active()
                    && self.workflows.active_kind.as_deref() == Some(&job.kind)
                    && job.state == "running"))
                && kinds.insert(job.kind.clone())
            {
                visible.push(job);
            }
        }
        visible.truncate(3);
        visible
    }

    pub fn workflow_history(&mut self, ui: &mut egui::Ui) {
        if self.workflows.jobs.is_empty() {
            return;
        }
        egui::CollapsingHeader::new(format!("历史任务 · {}", self.workflows.jobs.len()))
            .id_salt("workflow-history-logs")
            .show(ui, |ui| {
                let mut jobs = self.workflows.jobs.clone();
                jobs.sort_by_key(|job| std::cmp::Reverse(job.id));
                egui::ScrollArea::vertical()
                    .max_height(230.0)
                    .show(ui, |ui| {
                        for job in jobs {
                            ui.horizontal_wrapped(|ui| {
                                let kind = if job.kind.starts_with("dedup") {
                                    "去重"
                                } else if job.kind == "export" {
                                    "导出"
                                } else {
                                    "文本优化"
                                };
                                ui.label(format!("#{} {kind}", job.id));
                                badge(
                                    ui,
                                    if job.state == "completed" {
                                        "完成"
                                    } else {
                                        "未完成"
                                    },
                                    if job.state == "completed" {
                                        ACCENT
                                    } else {
                                        MUTED
                                    },
                                );
                                ui.label(format!("{} / {}", job.cursor, job.ids.len()));
                                if job.dedup_stats_recorded {
                                    icon_stat(
                                        ui,
                                        "trash",
                                        job.dedup_removed as u64,
                                        "自动移除",
                                        MUTED,
                                    );
                                }
                                if job.state != "completed"
                                    && !self.workflow_active()
                                    && !self.demo
                                    && !self.is_capture()
                                    && icon_button(ui, "forward", "继续此任务", false).clicked()
                                {
                                    if let Err(error) = self.start_workflow(
                                        ui.ctx(),
                                        &job.kind,
                                        Vec::new(),
                                        Some(job.id),
                                    ) {
                                        self.workflows.error = Some(error);
                                    }
                                }
                            });
                            if let Some(error) = job.error {
                                ui.add(
                                    egui::Label::new(RichText::new(error).color(AMBER).size(12.0))
                                        .wrap(),
                                );
                            }
                        }
                    });
            });
        ui.add_space(12.0);
    }

    fn pause_processing(&mut self) {
        self.workflows.auto_analysis_enabled = false;
        self.workflows.auto_analysis_waiting = false;
        if let Some(signal) = &self.workflows.cancel {
            signal.store(true, Ordering::Relaxed);
        }
    }

    pub fn character_relationships(&mut self, ui: &mut egui::Ui) {
        if self.demo && !self.is_capture() {
            return;
        }
        let editable =
            !self.demo && !self.is_capture() && !self.workflow_active() && !self.ingest_active();
        if editable && self.workflows.relationship_summary.is_none() {
            self.process_discovered_relationships(ui.ctx());
        }
        let editable = editable && !self.workflow_active();
        if let Some(error) = &self.workflows.error {
            ui.colored_label(AMBER, error);
        }
        let pending: Vec<_> = self
            .workflows
            .rules
            .characters
            .iter()
            .filter(|r| !r.verified)
            .cloned()
            .collect();
        if !pending.is_empty() {
            crate::review_ui::section().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new(format!("待确认角色关系 · {}", pending.len()))
                        .size(12.0)
                        .color(AMBER),
                );
                egui::ScrollArea::vertical()
                    .id_salt("character-pending-relations")
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for relation in &pending {
                            self.character_relation_row(ui, relation, editable);
                        }
                    });
            });
            ui.add_space(12.0);
        }
    }

    pub(crate) fn character_relations_dialog(&mut self, ui: &mut egui::Ui) -> bool {
        ui.set_width(600.0_f32.min(ui.ctx().screen_rect().width() - 96.0));
        let editable =
            !self.demo && !self.is_capture() && !self.workflow_active() && !self.ingest_active();
        if let Some(error) = &self.workflows.error {
            ui.colored_label(AMBER, error);
        }
        let confirmed = self
            .workflows
            .rules
            .characters
            .iter()
            .filter(|r| r.verified)
            .count();
        let pending: Vec<_> = self
            .workflows
            .rules
            .characters
            .iter()
            .filter(|r| !r.verified)
            .cloned()
            .collect();
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(format!("已生效 {confirmed} · 待确认 {}", pending.len()))
                    .size(12.0)
                    .color(MUTED),
            )
            .on_hover_text(
                "有效 Danbooru 关系自动处理；多个角色和具体变体仍保留。只有歧义关系需要确认。",
            );
            if ui
                .add_enabled(
                    editable,
                    egui::Button::new(egui::RichText::new("刷新来源关系").size(12.0)),
                )
                .on_hover_text("从 Danbooru 查询有效关系并自动处理。")
                .clicked()
            {
                if let Err(e) = self.discover_relationships(ui.ctx()) {
                    self.workflows.error = Some(e);
                }
            }
            let mut eligible = self.workflows.rules.clone();
            let count = eligible.process_discovered_characters();
            if count > 0
                && ui
                    .add_enabled(
                        editable,
                        egui::Button::new(format!("处理已发现关系 {count}")),
                    )
                    .on_hover_text("使用本地缓存，无需联网。")
                    .clicked()
            {
                self.process_discovered_relationships(ui.ctx());
            }
            if let Some(summary) = &self.workflows.relationship_summary {
                ui.label(egui::RichText::new("处理状态").size(12.0).color(MUTED))
                    .on_hover_text(summary);
            }
        });
        if !pending.is_empty() {
            ui.add_space(6.0);
            crate::review_ui::section().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new("待确认角色关系")
                        .size(12.0)
                        .color(AMBER),
                );
                egui::ScrollArea::vertical()
                    .id_salt("character-settings-pending-relations")
                    .max_height(100.0)
                    .show(ui, |ui| {
                        for relation in &pending {
                            self.character_relation_row(ui, relation, editable);
                        }
                    });
            });
        }
        ui.add_space(10.0);
        ui.label(egui::RichText::new("已生效关系").font(medium(13.0)));
        let relations: Vec<_> = self
            .workflows
            .rules
            .characters
            .iter()
            .filter(|r| r.verified)
            .cloned()
            .collect();
        if relations.is_empty() {
            ui.label(
                egui::RichText::new("暂无已生效关系。")
                    .size(12.0)
                    .color(MUTED),
            );
        } else {
            egui::ScrollArea::vertical()
                .id_salt("character-settings-confirmed-relations")
                .max_height(180.0)
                .show(ui, |ui| {
                    for relation in &relations {
                        self.character_relation_row(ui, relation, editable);
                    }
                });
        }
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("拒绝关系后，刷新也不会自动恢复；重新手动添加可恢复。")
                .size(12.0)
                .color(MUTED),
        );
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.workflows.base).hint_text("本体 Tag").desired_width(150.0));
            ui.add(egui::TextEdit::singleline(&mut self.workflows.variant).hint_text("变体 Tag").desired_width(150.0));
            if ui.add_enabled(editable, egui::Button::new("添加关系")).clicked() {
                let base = self.workflows.base.trim();
                let variant = self.workflows.variant.trim();
                if base != variant && self.workflows.rules.categories.get(base) == Some(&4)
                    && self.workflows.rules.categories.get(variant) == Some(&4) {
                    self.workflows.rules.rejected_characters.remove(&(base.into(), variant.into()));
                    self.workflows.rules.characters.push(CharacterRelation {
                        base: base.into(), variant: variant.into(), verified: true,
                        exclusive_group: String::new(), evidence: "human-confirmed".into(),
                    });
                    self.save_relationships(ui.ctx());
                } else {
                    self.workflows.error = Some("本体和变体须为两个已知 Danbooru 角色 Tag。请先添加收藏或下载包含这些角色的图片。".into());
                }
            }
        });
        false
    }

    fn character_relation_row(
        &mut self,
        ui: &mut egui::Ui,
        relation: &CharacterRelation,
        editable: bool,
    ) {
        let Some(index) = self
            .workflows
            .rules
            .characters
            .iter()
            .position(|r| r.base == relation.base && r.variant == relation.variant)
        else {
            return;
        };
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("{} → {}", relation.base, relation.variant))
                        .size(12.0),
                )
                .wrap(),
            );
            ui.label(
                egui::RichText::new(if relation.source_verified() {
                    "自动处理"
                } else if relation.verified {
                    "已确认"
                } else {
                    "待确认"
                })
                .size(12.0)
                .color(MUTED),
            )
            .on_hover_text(&relation.evidence);
            if !relation.verified
                && ui
                    .add_enabled(editable, egui::Button::new("确认关系"))
                    .clicked()
            {
                self.workflows.rules.characters[index].verified = true;
                self.workflows.rules.characters[index].evidence = "human-confirmed".into();
                self.save_relationships(ui.ctx());
            }
            if ui
                .add_enabled(editable, egui::Button::new("拒绝关系"))
                .on_hover_text("停止使用这条关系，并记住拒绝决定；不删除图片。")
                .clicked()
            {
                self.workflows
                    .rules
                    .rejected_characters
                    .insert((relation.base.clone(), relation.variant.clone()));
                self.workflows.rules.characters.remove(index);
                self.save_relationships(ui.ctx());
            }
        });
    }

    fn process_discovered_relationships(&mut self, ctx: &egui::Context) {
        if self.demo || self.is_capture() || self.workflow_active() || self.ingest_active() {
            return;
        }
        let original = self.workflows.rules.clone();
        let count = self.workflows.rules.process_discovered_characters();
        if count == 0 {
            return;
        }
        self.workflows.error = None;
        self.save_relationships(ctx);
        if self.workflows.error.is_some() {
            self.workflows.rules = self
                .workflow_repo()
                .and_then(|repo| repo.tag_rules().map_err(|e| e.to_string()))
                .unwrap_or(original);
            self.workflows.relationship_summary = Some("自动处理未完成，请重试。".into());
        } else {
            self.workflows.relationship_summary =
                Some(format!("已自动处理 {count} 条来源关系，正在更新本地标签。"));
        }
    }

    pub(crate) fn save_relationships(&mut self, ctx: &egui::Context) {
        self.tags.invalidate();
        if self.demo || self.is_capture() || self.workflow_active() || self.ingest_active() {
            return;
        }
        if let Err(e) = self.workflows.rules.validate_characters() {
            self.workflows.error = Some(e.into());
            if let Ok(repo) = self.workflow_repo() {
                self.workflows.rules = repo.tag_rules().unwrap_or_default();
            }
            return;
        }
        let result = (|| {
            let mut repo = self.workflow_repo()?;
            repo.save_tag_rules(&self.workflows.rules)
                .map_err(|e| e.to_string())?;
            let (sender, receiver) = mpsc::channel();
            let repaint = ctx.clone();
            std::thread::Builder::new()
                .name("local-tag-normalization".into())
                .spawn(move || {
                    let result = (|| {
                        for media in repo.list_media().map_err(|e| e.to_string())? {
                            repo.normalize_media(&media).map_err(|e| e.to_string())?;
                        }
                        Ok(())
                    })();
                    let _ = sender.send(Event::Finished(result));
                    repaint.request_repaint();
                })
                .map_err(|_| "无法启动本地标签更新。")?;
            self.workflows.pending = Some(receiver);
            self.workflows.metadata_active = true;
            Ok::<_, String>(())
        })();
        if let Err(e) = result {
            self.workflows.error = Some(e);
        } else {
            let _ = self.reload_workflow_data();
        }
    }
    fn discover_relationships(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.demo || self.is_capture() || self.workflow_active() || self.ingest_active() {
            return Err("请等待当前任务完成后再发现关系。".into());
        }
        let credentials = self
            .saved_danbooru_credentials()
            .map_err(|e| e.to_string())?;
        let mut repo = self.workflow_repo()?;
        let mut rules = self.workflows.rules.clone();
        rules.process_discovered_characters();
        for c in &self.library.characters {
            rules.categories.insert(c.tag.clone(), 4);
        }
        let names: Vec<_> = rules
            .categories
            .iter()
            .filter(|(_, cat)| **cat == 4)
            .map(|(tag, _)| tag.clone())
            .collect();
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name("character-relations".into())
            .spawn(move || {
                use bdm_ports::TagProvider;
                let result = (|| {
                    let provider = bdm_infrastructure::danbooru::DanbooruProvider::new(credentials)
                        .map_err(|e| e.to_string())?;
                    for name in names {
                        for base in provider
                            .character_implications(&name)
                            .map_err(|e| e.to_string())?
                        {
                            if !rules
                                .rejected_characters
                                .contains(&(base.clone(), name.clone()))
                                && provider
                                    .lookup(&base)
                                    .map_err(|e| e.to_string())?
                                    .is_some_and(|tag| tag.category == 4)
                                && !rules
                                    .characters
                                    .iter()
                                    .any(|r| r.base == base && r.variant == name)
                            {
                                rules.categories.insert(base.clone(), 4);
                                rules.accept_character_implication(&base, &name);
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(250));
                    }
                    rules.process_discovered_characters();
                    repo.save_tag_rules(&rules).map_err(|e| e.to_string())?;
                    for media in repo.list_media().map_err(|e| e.to_string())? {
                        repo.normalize_media(&media).map_err(|e| e.to_string())?;
                    }
                    let _ = sender.send(Event::Rules(rules));
                    Ok(())
                })();
                let _ = sender.send(Event::Finished(result));
                repaint.request_repaint();
            })
            .map_err(|_| "无法启动关系发现。")?;
        self.workflows.pending = Some(receiver);
        self.workflows.metadata_active = true;
        Ok(())
    }

    pub fn resolve_tags(&mut self, ctx: &egui::Context) -> Result<(), String> {
        self.resolve_tags_cached(ctx, true)
    }
    fn resolve_tags_cached(&mut self, ctx: &egui::Context, refresh: bool) -> Result<(), String> {
        if self.workflow_active() || self.ingest_active() || self.demo || self.is_capture() {
            return Err("当前不能查询标签。".into());
        }
        let credentials = self
            .saved_danbooru_credentials()
            .map_err(|e| e.to_string())?;
        let mut repo = self.workflow_repo()?;
        let mut rules = self.workflows.rules.clone();
        rules.process_discovered_characters();
        if refresh {
            rules.resolved_aliases.clear();
            rules.discovered_characters.clear();
        }
        let names: BTreeSet<_> = self
            .library
            .media
            .iter()
            .flat_map(|m| m.tags.iter().cloned())
            .collect();
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name("tag-normalization".into())
            .spawn(move || {
                use bdm_ports::TagProvider;
                let result = (|| {
                    let provider = bdm_infrastructure::danbooru::DanbooruProvider::new(credentials)
                        .map_err(|e| e.to_string())?;
                    for name in names {
                        if rules.resolved_aliases.contains(&name) {
                            continue;
                        }
                        if let Some(canonical) =
                            provider.canonical_alias(&name).map_err(|e| e.to_string())?
                        {
                            if let Some(tag) =
                                provider.lookup(&canonical).map_err(|e| e.to_string())?
                            {
                                rules.categories.insert(tag.name.clone(), tag.category);
                                rules.aliases.insert(name.clone(), tag.name);
                            }
                        } else if !rules.categories.contains_key(&name) {
                            if let Some(tag) = provider.lookup(&name).map_err(|e| e.to_string())? {
                                rules.categories.insert(tag.name, tag.category);
                            }
                        }
                        rules.resolved_aliases.insert(name);
                        repo.save_tag_rules(&rules).map_err(|e| e.to_string())?;
                        std::thread::sleep(std::time::Duration::from_millis(250));
                    }
                    let characters: Vec<_> = rules
                        .categories
                        .iter()
                        .filter(|(_, cat)| **cat == 4)
                        .map(|(name, _)| name.clone())
                        .collect();
                    for name in characters {
                        if rules.discovered_characters.contains(&name) {
                            continue;
                        }
                        for base in provider
                            .character_implications(&name)
                            .map_err(|e| e.to_string())?
                        {
                            if rules
                                .rejected_characters
                                .contains(&(base.clone(), name.clone()))
                                || rules
                                    .characters
                                    .iter()
                                    .any(|r| r.base == base && r.variant == name)
                            {
                                continue;
                            }
                            if provider
                                .lookup(&base)
                                .map_err(|e| e.to_string())?
                                .is_some_and(|tag| tag.category == 4)
                            {
                                rules.categories.insert(base.clone(), 4);
                                rules.accept_character_implication(&base, &name);
                            }
                        }
                        rules.discovered_characters.insert(name);
                        repo.save_tag_rules(&rules).map_err(|e| e.to_string())?;
                        std::thread::sleep(std::time::Duration::from_millis(250));
                    }
                    for media in repo.list_media().map_err(|e| e.to_string())? {
                        repo.normalize_media(&media).map_err(|e| e.to_string())?;
                    }
                    let _ = sender.send(Event::Rules(rules));
                    Ok(())
                })();
                let _ = sender.send(Event::Finished(result));
                repaint.request_repaint();
            })
            .map_err(|_| "无法启动标签解析。")?;
        self.workflows.pending = Some(receiver);
        self.workflows.metadata_active = true;
        Ok(())
    }

    fn similarity_candidates(&self) -> Vec<DuplicateCandidate> {
        let scope: BTreeSet<_> = self.action_ids().into_iter().collect();
        let mut candidates: Vec<_> = self
            .workflows
            .candidates
            .iter()
            .filter(|p| {
                p.state == "pending" && scope.contains(&p.first) && scope.contains(&p.second)
            })
            .cloned()
            .collect();
        candidates.sort_by(|a, b| {
            b.cosine
                .unwrap_or(-1.0)
                .total_cmp(&a.cosine.unwrap_or(-1.0))
                .then_with(|| a.first.cmp(&b.first))
                .then_with(|| a.second.cmp(&b.second))
        });
        candidates
    }

    pub fn candidate_review(&mut self, ui: &mut egui::Ui) {
        let candidates = self.similarity_candidates();
        let pages = candidates.len().div_ceil(5).max(1);
        self.workflows.candidate_page = self.workflows.candidate_page.min(pages - 1);
        if candidates.is_empty() {
            return;
        }
        ui.horizontal(|ui| {
            badge(ui, &candidates.len().to_string(), ACCENT);
            if primary(ui, "看图审核").clicked() {
                self.open_scoped_pair_review(&candidates[0]);
            }
            if ui
                .add_enabled_ui(self.workflows.candidate_page > 0, |ui| {
                    icon_button(ui, "back", "上一页", false)
                })
                .inner
                .clicked()
            {
                self.workflows.candidate_page -= 1;
            }
            if ui
                .add_enabled_ui(self.workflows.candidate_page + 1 < pages, |ui| {
                    icon_button(ui, "forward", "下一页", false)
                })
                .inner
                .clicked()
            {
                self.workflows.candidate_page += 1;
            }
        });
        egui::ScrollArea::vertical()
            .id_salt(("similarity-candidate-pages", self.workflows.candidate_page))
            .max_height(250.0)
            .show(ui, |ui| {
                for pair in candidates
                    .into_iter()
                    .skip(self.workflows.candidate_page * 5)
                    .take(5)
                {
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - 8.0) * 0.5;
                        for id in [pair.first, pair.second] {
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(width, 145.0),
                                egui::Sense::click(),
                            );
                            self.paint_media(ui, rect, id as usize);
                            if response.on_hover_text("看图审核").clicked() {
                                self.open_scoped_pair_review(&pair);
                            }
                        }
                    });
                    ui.add_space(8.0);
                }
            });
    }
}
