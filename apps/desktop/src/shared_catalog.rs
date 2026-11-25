use crate::{paths, thumbnails::Thumbnails};
use bdm_config::ModelScopeConfig;
use bdm_infrastructure::modelscope;
use eframe::egui;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, TryRecvError},
};

#[derive(Clone, Deserialize)]
struct Entry {
    id: String,
    name: String,
    labels: String,
    caption: String,
    collection: String,
    review: String,
    cached: bool,
    cached_path: Option<PathBuf>,
}

#[derive(Deserialize)]
struct Commit {
    id: String,
    message: String,
}

#[derive(Deserialize)]
struct ConflictVersion {
    name: String,
    labels: String,
    caption: String,
    collection: String,
    review: String,
    original: Original,
}

#[derive(Deserialize)]
struct Original {
    sha256: String,
}

#[derive(Deserialize)]
struct Conflict {
    id: String,
    local: Option<ConflictVersion>,
    remote: Option<ConflictVersion>,
}

#[derive(Deserialize)]
struct EntryRef {
    id: String,
}

#[derive(Deserialize)]
struct Change {
    id: String,
    change: String,
    name: Option<String>,
}

#[derive(Deserialize)]
struct CommitDiff {
    commit_id: String,
    changes: Vec<Change>,
}

#[derive(Default, Deserialize)]
struct View {
    ahead: usize,
    behind: usize,
    changes: Vec<Change>,
    merge_in_progress: bool,
    conflict_details: Vec<Conflict>,
    entries: Vec<Entry>,
    commits: Vec<Commit>,
}

#[derive(Deserialize)]
struct SyncSummary {
    status: String,
    exported: usize,
    imported: usize,
    updated: usize,
    conflicts: usize,
}

#[derive(Deserialize)]
struct Response {
    view: View,
    path: Option<PathBuf>,
    entry: Option<EntryRef>,
    diff: Option<CommitDiff>,
    sync: Option<SyncSummary>,
}

#[derive(Default)]
pub struct SharedCatalog {
    open: bool,
    config: Option<ModelScopeConfig>,
    pending: Option<(&'static str, Receiver<Result<Response, String>>)>,
    error: Option<String>,
    sync_status: Option<String>,
    sync_reload: bool,
    view: View,
    query: String,
    selection: Option<String>,
    file: String,
    name: String,
    labels: String,
    caption: String,
    collection: String,
    review: String,
    message: String,
    media_id: String,
    preview_path: Option<PathBuf>,
    previews: Thumbnails,
    textures: BTreeMap<usize, egui::TextureHandle>,
    diff: Option<CommitDiff>,
}

impl SharedCatalog {
    pub fn toggle(&mut self, ctx: &egui::Context, config: ModelScopeConfig) {
        if self.busy() {
            return;
        }
        if self.open && self.config.as_ref() == Some(&config) {
            self.open = false;
            return;
        }
        if self.config.as_ref() != Some(&config) {
            *self = Self::default();
            self.config = Some(config);
        }
        self.open = true;
        self.start(ctx, "view", json!({}));
    }

    pub fn sync(
        &mut self,
        ctx: &egui::Context,
        config: ModelScopeConfig,
        catalog: PathBuf,
        library: PathBuf,
    ) {
        if self.busy() {
            return;
        }
        if self.config.as_ref() != Some(&config) {
            let open = self.open;
            *self = Self::default();
            self.open = open;
            self.config = Some(config);
        }
        self.sync_status = Some("正在同步图库进度...".into());
        self.start(
            ctx,
            "sync",
            json!({ "catalog": catalog, "library": library }),
        );
    }

    pub fn sync_status(&self) -> Option<&str> {
        self.sync_status.as_deref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn take_sync_reload(&mut self) -> bool {
        std::mem::take(&mut self.sync_reload)
    }

    fn start(&mut self, ctx: &egui::Context, action: &'static str, args: Value) {
        let Some(config) = self.config.clone() else {
            return;
        };
        if self.pending.is_some() {
            return;
        }
        let (send, pending) = mpsc::channel();
        let repaint = ctx.clone();
        let worker = std::thread::Builder::new()
            .name("modelscope-catalog".into())
            .spawn(move || {
                let root = paths::project_root();
                let workspace = root.join("data/modelscope-catalog");
                let result = modelscope::shared_catalog(&root, &config, &workspace, action, &args);
                let _ = send.send(result);
                repaint.request_repaint();
            });
        match worker {
            Ok(_) => {
                self.error = None;
                self.pending = Some((action, pending));
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn poll(&mut self) {
        let Some((action, pending)) = &self.pending else {
            return;
        };
        let result = match pending.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("魔搭共享图库操作中断。".into()),
        };
        let action = *action;
        self.pending = None;
        match result {
            Ok(response) => {
                self.view = response.view;
                let current = self.selection.as_deref();
                let saved = response.entry.as_ref().map(|entry| entry.id.as_str());
                if let Some(id) = saved.or(current.filter(|_| {
                    matches!(action, "view" | "pull" | "delete" | "resolve" | "revert")
                })) {
                    let entry = self
                        .view
                        .entries
                        .iter()
                        .find(|entry| entry.id == id)
                        .cloned();
                    self.select(entry);
                }
                if action == "commit" {
                    self.message.clear();
                }
                if response.diff.is_some() {
                    self.diff = response.diff;
                }
                if let Some((path, error)) = response
                    .path
                    .and_then(|path| open_original(&path).err().map(|error| (path, error)))
                {
                    self.error = Some(format!("{} ({})", error, path.display()));
                }
                if let Some(sync) = response.sync {
                    if sync.status == "conflicts" || sync.conflicts > 0 {
                        self.open = true;
                        self.sync_status = Some(format!(
                            "同步发现 {} 个冲突，请在下方选择保留版本后再次同步。",
                            sync.conflicts
                        ));
                    } else {
                        self.sync_status = Some(format!(
                            "同步完成：上传 {} · 新增 {} · 更新 {}",
                            sync.exported, sync.imported, sync.updated
                        ));
                        if sync.imported > 0 || sync.updated > 0 {
                            self.sync_reload = true;
                        }
                    }
                }
            }
            Err(error) => {
                self.sync_status = None;
                self.error = Some(error);
            }
        }
    }

    fn select(&mut self, entry: Option<Entry>) {
        self.file.clear();
        if let Some(entry) = entry {
            self.selection = Some(entry.id);
            self.name = entry.name;
            self.labels = entry.labels;
            self.caption = entry.caption;
            self.collection = entry.collection;
            self.review = entry.review;
        } else {
            self.selection = None;
            self.name.clear();
            self.labels.clear();
            self.caption.clear();
            self.collection.clear();
            self.review = "unreviewed".into();
        }
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn visible(&self) -> bool {
        self.open
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        if self.busy() && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        self.poll();
        self.previews.poll(ctx, &mut self.textures);
    }

    pub fn show_inline(&mut self, ui: &mut egui::Ui) {
        if !self.open {
            return;
        }
        let title = self.config.as_ref().map_or_else(
            || "魔搭共享图库".to_owned(),
            |config| format!("魔搭共享图库 · {}", config.dataset_id),
        );
        ui.add_space(16.0);
        crate::theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            crate::widgets::setting_header(ui, "lock", &title);
            if self.busy() {
                ui.spinner();
            }
            if let Some(error) = &self.error {
                ui.label(error);
            }
            ui.add_enabled_ui(!self.busy(), |ui| self.contents(ui));
        });
    }

    fn contents(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (action, label) in [("view", "刷新本地"), ("pull", "拉取"), ("push", "推送")]
            {
                if ui.button(label).clicked() {
                    self.start(ui.ctx(), action, json!({}));
                }
            }
            ui.label(format!(
                "{} 个条目待提交 · {} 个提交待推送 · 落后 {} 个已知提交",
                self.view.changes.len(),
                self.view.ahead,
                self.view.behind
            ));
        });
        ui.horizontal_wrapped(|ui| {
            ui.text_edit_singleline(&mut self.message)
                .on_hover_text("提交说明");
            let ready = (!self.view.changes.is_empty() || self.view.merge_in_progress)
                && self.view.conflict_details.is_empty()
                && !self.message.trim().is_empty();
            if ui.add_enabled(ready, egui::Button::new("提交")).clicked() {
                self.start(ui.ctx(), "commit", json!({ "message": self.message }));
            }
        });
        ui.separator();
        self.show_conflicts(ui);
        if !self.view.changes.is_empty() {
            egui::CollapsingHeader::new("待提交变更")
                .show(ui, |ui| self.show_changes(ui, &self.view.changes));
        }
        ui.columns(2, |columns| {
            self.show_entries(&mut columns[0]);
            self.show_editor(&mut columns[1]);
        });
        ui.separator();
        self.show_history(ui);
    }

    fn show_conflicts(&mut self, ui: &mut egui::Ui) {
        let mut choice = None;
        for conflict in &self.view.conflict_details {
            ui.label(format!("冲突条目 {}", conflict.id));
            ui.horizontal_wrapped(|ui| {
                for (side, label) in [("local", "保留本机"), ("remote", "采用远端")] {
                    let version = if side == "local" {
                        &conflict.local
                    } else {
                        &conflict.remote
                    };
                    let details = version.as_ref().map_or_else(
                        || "已删除".to_owned(),
                        |entry| {
                            format!(
                                "{}\n标签 {}\n描述 {}\n合集 {}\n审核 {}\n原图 {}",
                                entry.name,
                                entry.labels,
                                entry.caption,
                                entry.collection,
                                entry.review,
                                entry.original.sha256
                            )
                        },
                    );
                    ui.label(details);
                    if ui.button(label).clicked() {
                        choice = Some((conflict.id.clone(), side));
                    }
                }
            });
        }
        if let Some((id, side)) = choice {
            self.start(ui.ctx(), "resolve", json!({ "entry_id": id, "side": side }));
        }
    }

    fn show_entries(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("搜索名称、标签或合集"));
            if ui.button("新增").clicked() {
                self.select(None);
            }
        });
        let query = self.query.trim().to_lowercase();
        let entries = self
            .view
            .entries
            .iter()
            .filter(|entry| {
                [
                    &entry.name,
                    &entry.labels,
                    &entry.caption,
                    &entry.collection,
                ]
                .iter()
                .any(|field| field.to_lowercase().contains(&query))
            })
            .collect::<Vec<_>>();
        let mut selection = None;
        egui::ScrollArea::vertical()
            .id_salt("shared-entries")
            .max_height(310.0)
            .show_rows(ui, 24.0, entries.len(), |ui, rows| {
                for index in rows {
                    let entry = entries[index];
                    let label = if entry.cached {
                        entry.name.clone()
                    } else {
                        format!("{} · 按需下载", entry.name)
                    };
                    if ui
                        .selectable_label(self.selection.as_ref() == Some(&entry.id), label)
                        .clicked()
                    {
                        selection = Some(entry.clone());
                    }
                }
            });
        if selection.is_some() {
            self.select(selection);
        }
    }

    fn show_editor(&mut self, ui: &mut egui::Ui) {
        let id = self.selection.clone();
        self.show_preview(ui);
        ui.label(if id.is_some() {
            "编辑条目"
        } else {
            "新增条目"
        });
        for (label, value) in [
            ("名称", &mut self.name),
            ("标签", &mut self.labels),
            ("合集", &mut self.collection),
            ("原图文件路径", &mut self.file),
        ] {
            ui.label(label);
            ui.text_edit_singleline(value);
        }
        ui.label("描述");
        ui.add(egui::TextEdit::multiline(&mut self.caption).desired_rows(2));
        let reviews = [
            ("unreviewed", "待审"),
            ("approved", "通过"),
            ("rejected", "不通过"),
        ];
        egui::ComboBox::from_id_salt("shared-review")
            .selected_text(
                reviews
                    .iter()
                    .find(|(value, _)| *value == self.review)
                    .map_or("待审", |(_, label)| label),
            )
            .show_ui(ui, |ui| {
                for (value, label) in reviews {
                    ui.selectable_value(&mut self.review, value.to_owned(), label);
                }
            });
        ui.horizontal_wrapped(|ui| {
            let editable = self.view.conflict_details.is_empty();
            if ui
                .add_enabled(
                    editable && (id.is_some() || !self.file.trim().is_empty()),
                    egui::Button::new("保存到本地"),
                )
                .clicked()
            {
                let mut args = json!({
                    "name": self.name,
                    "labels": self.labels,
                    "caption": self.caption,
                    "collection": self.collection,
                    "review": if self.review.is_empty() { "unreviewed" } else { &self.review },
                    "file": self.file.trim(),
                });
                let action = if let Some(id) = &id {
                    args["entry_id"] = json!(id);
                    "edit"
                } else {
                    "add"
                };
                self.start(ui.ctx(), action, args);
            }
            if let Some(id) = id {
                if ui.button("打开原图").clicked() {
                    self.start(ui.ctx(), "original", json!({ "entry_id": id }));
                }
                if ui
                    .add_enabled(
                        self.preview_path.is_some(),
                        egui::Button::new("清理本机缓存"),
                    )
                    .clicked()
                {
                    self.start(ui.ctx(), "clear-cache", json!({ "entry_id": id }));
                }
                if ui
                    .add_enabled(editable, egui::Button::new("删除条目"))
                    .clicked()
                {
                    self.start(ui.ctx(), "delete", json!({ "entry_id": id }));
                }
            }
        });
        if self.selection.is_none() {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.media_id).hint_text("本机 BDM 图片 ID"),
                );
                let id = self
                    .media_id
                    .trim()
                    .parse::<i64>()
                    .ok()
                    .filter(|id| *id > 0);
                if let Some(id) = id.filter(|_| self.view.conflict_details.is_empty()) {
                    if ui.button("导入已验证原图").clicked() {
                        let catalog = paths::data_directory().join("catalog.sqlite3");
                        self.start(
                            ui.ctx(),
                            "import",
                            json!({ "catalog": catalog, "media_id": id }),
                        );
                    }
                }
            });
        }
    }

    fn show_preview(&mut self, ui: &mut egui::Ui) {
        let path = self
            .view
            .entries
            .iter()
            .find(|entry| self.selection.as_ref() == Some(&entry.id))
            .and_then(|entry| entry.cached_path.clone());
        if self.preview_path != path {
            self.preview_path = path;
            self.previews.invalidate(0);
            self.textures.remove(&0);
        }
        if let Some(texture) = self.textures.get(&0) {
            ui.add(egui::Image::new(texture).max_size(egui::vec2(180.0, 150.0)));
        } else if let Some(path) = &self.preview_path {
            self.previews.request_preview(ui.ctx(), 0, path.clone());
        }
    }

    fn show_history(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        egui::CollapsingHeader::new("提交历史").show(ui, |ui| {
            for commit in &self.view.commits {
                ui.horizontal_wrapped(|ui| {
                    ui.monospace(commit.id.chars().take(8).collect::<String>());
                    ui.label(&commit.message);
                    if ui.button("查看变更").clicked() {
                        action = Some(("inspect", commit.id.clone()));
                    }
                    if ui.button("撤销此提交").clicked() {
                        action = Some(("revert", commit.id.clone()));
                    }
                });
                if let Some(diff) = self
                    .diff
                    .as_ref()
                    .filter(|diff| diff.commit_id == commit.id)
                {
                    self.show_changes(ui, &diff.changes);
                }
            }
        });
        if let Some((action, commit)) = action {
            self.start(ui.ctx(), action, json!({ "commit_id": commit }));
        }
    }

    fn show_changes(&self, ui: &mut egui::Ui, changes: &[Change]) {
        for change in changes {
            let kind = match change.change.as_str() {
                "A" | "??" => "新增",
                "M" | "AM" => "修改",
                "D" => "删除",
                _ => "变更",
            };
            let name = change
                .name
                .as_deref()
                .or_else(|| {
                    self.view
                        .entries
                        .iter()
                        .find(|entry| entry.id == change.id)
                        .map(|entry| entry.name.as_str())
                })
                .unwrap_or(&change.id);
            ui.label(format!("{kind} · {name}"));
        }
    }
}

fn open_original(path: &Path) -> Result<(), String> {
    let viewer = if cfg!(windows) {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(viewer)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|_| "已下载原图，无法启动图片查看器。".into())
}
