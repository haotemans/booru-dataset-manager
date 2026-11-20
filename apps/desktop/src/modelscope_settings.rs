use crate::{
    app::BdmApp,
    paths,
    theme::{card, MUTED},
    widgets::setting_header,
};
use bdm_config::{ModelScopeConfig, ModelScopeSite};
use bdm_infrastructure::{
    credentials::NativeCredentialStore,
    modelscope::{self, Transfer, TransferResult},
};
use bdm_ports::{SourceCredentialStore, SourceCredentials};
use eframe::egui::{self, RichText};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::mpsc::{self, Receiver, TryRecvError},
};

#[derive(Default)]
pub struct ModelScopeSettings {
    dataset: String,
    site: ModelScopeSite,
    token: String,
    saved_dataset: String,
    token_saved: bool,
    error: Option<String>,
    status: Option<String>,
    pending: Option<Receiver<Result<TransferResult, String>>>,
    received_window: Option<(PathBuf, Child)>,
}

impl ModelScopeSettings {
    pub fn load(&mut self, root: &Path) {
        let result = (|| -> Result<(), String> {
            if let Some(config) = modelscope::load_config(root).map_err(|e| e.to_string())? {
                self.dataset.clone_from(&config.dataset_id);
                self.site = config.site.unwrap_or_default();
                self.saved_dataset.clone_from(&config.dataset_id);
                self.token_saved = NativeCredentialStore::modelscope(&config.dataset_id)
                    .load()
                    .map_err(|e| e.to_string())?
                    .is_some_and(|value| value.username == config.dataset_id);
            }
            Ok(())
        })();
        self.error = result.err();
    }

    fn save(&mut self, root: &Path) -> Result<(), String> {
        let mut config = ModelScopeConfig::new(&self.dataset)?;
        config.site = Some(self.site);
        let token = self.token.trim();
        if token.len() > 1024 || token.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("Access Token 格式无效。".into());
        }
        let store = NativeCredentialStore::modelscope(&config.dataset_id);
        let token_saved = if token.is_empty() {
            store
                .load()
                .map_err(|e| e.to_string())?
                .is_some_and(|value| value.username == config.dataset_id)
        } else {
            store
                .save(&SourceCredentials {
                    username: config.dataset_id.clone(),
                    api_key: token.to_owned(),
                })
                .map_err(|e| e.to_string())?;
            true
        };
        modelscope::save_config(root, &config).map_err(|e| e.to_string())?;
        self.dataset.clone_from(&config.dataset_id);
        self.saved_dataset = config.dataset_id;
        self.token_saved = token_saved;
        self.token.clear();
        self.error = None;
        self.status = None;
        Ok(())
    }

    fn clear_token(&mut self) -> Result<(), String> {
        NativeCredentialStore::modelscope(&self.saved_dataset)
            .clear()
            .map_err(|e| e.to_string())?;
        self.token.clear();
        self.token_saved = false;
        self.error = None;
        self.status = None;
        Ok(())
    }

    fn start(&mut self, ctx: &egui::Context, transfer: Transfer) -> Result<(), String> {
        let root = paths::project_root();
        self.save(&root)?;
        if !self.token_saved {
            return Err("请先填写魔搭 Access Token。".into());
        }
        let message = match transfer {
            Transfer::Upload { .. } => "正在上传并校验本机图库…",
            Transfer::Receive { .. } => "正在下载并校验最新图库…",
        };
        let (sender, receiver) = mpsc::channel();
        let site = self.site;
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name("modelscope-transfer".into())
            .spawn(move || {
                let _ = sender.send(modelscope::transfer(&root, site, transfer));
                repaint.request_repaint();
            })
            .map_err(|_| "无法启动魔搭任务。".to_owned())?;
        self.status = Some(message.into());
        self.pending = Some(receiver);
        Ok(())
    }

    fn open_received(&mut self, directory: PathBuf) -> Result<(), String> {
        if let Some((open_directory, child)) = &mut self.received_window {
            if *open_directory == directory && matches!(child.try_wait(), Ok(None)) {
                return Ok(());
            }
        }
        let executable = std::env::current_exe().map_err(|_| "找不到当前 BDM 程序。")?;
        let child = Command::new(executable)
            .arg("--received-data")
            .arg(&directory)
            .spawn()
            .map_err(|_| {
                format!(
                    "图库已下载，但无法打开 BDM 窗口。目录：{}",
                    directory.display()
                )
            })?;
        self.received_window = Some((directory, child));
        Ok(())
    }
}

impl BdmApp {
    pub fn poll_modelscope_transfer(&mut self, ctx: &egui::Context) {
        let settings = &mut self.modelscope;
        let Some(receiver) = &settings.pending else {
            return;
        };
        if ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("魔搭任务中断。".into()),
        };
        settings.pending = None;
        settings.status = None;
        match result {
            Ok(result) => {
                if let Some(directory) = result.destination {
                    match settings.open_received(directory) {
                        Ok(()) => {
                            settings.status = Some(format!("已打开接收版本 {}", result.backup_id))
                        }
                        Err(error) => settings.error = Some(error),
                    }
                } else {
                    settings.status =
                        Some(format!("上传校验完成，B 机可下载版本 {}", result.backup_id));
                }
            }
            Err(error) => settings.error = Some(error),
        }
    }

    pub fn modelscope_settings_card(&mut self, ui: &mut egui::Ui) {
        let available = !self.demo && !self.is_capture();
        let idle = !self.ingest_active() && !self.workflow_active() && !self.image_editor.active();
        let mut saved = false;
        let mut cleared = false;
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            setting_header(ui, "lock", "魔搭备份");
            ui.add_space(8.0);
            let settings = &mut self.modelscope;
            let enabled = available && settings.pending.is_none();
            let field_width = ui.available_width().min(360.0);
            ui.label("魔搭站点");
            ui.add_enabled_ui(enabled, |ui| {
                egui::ComboBox::from_id_salt("modelscope-site")
                    .selected_text(settings.site.endpoint())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut settings.site,
                            ModelScopeSite::China,
                            "中国站 · modelscope.cn",
                        );
                        ui.selectable_value(
                            &mut settings.site,
                            ModelScopeSite::International,
                            "国际站 · modelscope.ai",
                        );
                    });
            });
            ui.label("私有数据集 ID");
            ui.add_enabled(
                enabled,
                egui::TextEdit::singleline(&mut settings.dataset)
                    .hint_text("用户名/数据集名")
                    .desired_width(field_width),
            );
            ui.label("Access Token");
            let has_token =
                settings.token_saved && settings.dataset.trim() == settings.saved_dataset;
            ui.add_enabled(
                enabled,
                egui::TextEdit::singleline(&mut settings.token)
                    .password(true)
                    .hint_text(if has_token {
                        "已保存，留空保留"
                    } else {
                        "输入令牌"
                    })
                    .desired_width(field_width),
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(enabled, egui::Button::new("保存魔搭设置"))
                    .clicked()
                {
                    match settings.save(&paths::project_root()) {
                        Ok(()) => saved = true,
                        Err(error) => settings.error = Some(error),
                    }
                }
                if ui
                    .add_enabled(enabled && has_token, egui::Button::new("移除令牌"))
                    .clicked()
                {
                    match settings.clear_token() {
                        Ok(()) => cleared = true,
                        Err(error) => settings.error = Some(error),
                    }
                }
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(enabled && idle, egui::Button::new("A 机：上传本机图库"))
                    .on_disabled_hover_text("等当前图库任务结束后再上传。")
                    .clicked()
                {
                    let transfer = Transfer::Upload {
                        data: paths::data_directory(),
                        library: PathBuf::from(&self.prefs.root),
                    };
                    if let Err(error) = settings.start(ui.ctx(), transfer) {
                        settings.error = Some(error);
                    }
                }
                if ui
                    .add_enabled(
                        enabled && settings.pending.is_none(),
                        egui::Button::new("B 机：下载并打开"),
                    )
                    .clicked()
                {
                    let transfer = Transfer::Receive {
                        directory: paths::project_root().join("dist/modelscope-received"),
                    };
                    if let Err(error) = settings.start(ui.ctx(), transfer) {
                        settings.error = Some(error);
                    }
                }
            });
            if let Some(status) = &settings.status {
                ui.horizontal_wrapped(|ui| {
                    if settings.pending.is_some() {
                        ui.spinner();
                    }
                    ui.label(status);
                });
            }
            if let Some(error) = &settings.error {
                ui.label(RichText::new(error).color(MUTED));
            }
        });
        if saved {
            self.notify(ui.ctx(), "魔搭设置已保存。");
        } else if cleared {
            self.notify(ui.ctx(), "已移除本机保存的魔搭令牌。");
        }
    }
}
