use crate::{app::BdmApp, theme::*, widgets::*};
use bdm_ports::{CatalogError, SourceCredentialStore, SourceCredentials};
use eframe::egui::{self, vec2, RichText};
use std::sync::mpsc::{self, Receiver, TryRecvError};

struct PendingVerification {
    generation: u64,
    receiver: Receiver<Result<(), CatalogError>>,
}

pub struct DanbooruSettings {
    pub username: String,
    // This type is never serialized, and the key is never copied into Preferences.
    pub api_key: String,
    pub saved_username: Option<String>,
    pub error: Option<String>,
    pub store: Box<dyn SourceCredentialStore>,
    pub verification: Option<Result<(), CatalogError>>,
    generation: u64,
    pending: Option<PendingVerification>,
}

impl DanbooruSettings {
    pub fn new(username: String) -> Self {
        Self {
            username,
            api_key: String::new(),
            saved_username: None,
            error: None,
            store: Box::new(bdm_infrastructure::credentials::NativeCredentialStore::danbooru()),
            verification: None,
            generation: 0,
            pending: None,
        }
    }
}

impl BdmApp {
    pub fn saved_danbooru_credentials(&self) -> Result<Option<SourceCredentials>, CatalogError> {
        if let Some(username) = &self.danbooru.saved_username {
            let credentials = self
                .danbooru
                .store
                .load()
                .map_err(|_| CatalogError::Credentials)?
                .ok_or(CatalogError::Credentials)?;
            if &credentials.username != username
                || bdm_application::validate_source_credentials(
                    &credentials.username,
                    &credentials.api_key,
                )
                .is_err()
            {
                return Err(CatalogError::Credentials);
            }
            Ok(Some(credentials))
        } else if self.danbooru.error.is_some() && !self.prefs.danbooru_username.is_empty() {
            Err(CatalogError::Credentials)
        } else {
            Ok(None)
        }
    }

    pub fn start_danbooru_verification(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.demo || self.is_capture() {
            return Err("示例预览不验证远程账号。".into());
        }
        if self.danbooru.pending.is_some() {
            return Err("正在验证连接，请稍后。".into());
        }
        let credentials = self
            .saved_danbooru_credentials()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "请先保存 Danbooru 用户名和 API Key。".to_owned())?;
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name("danbooru-account-verification".into())
            .spawn(move || {
                let result = bdm_infrastructure::danbooru::DanbooruProvider::new(Some(credentials))
                    .and_then(|provider| provider.verify_credentials());
                let _ = sender.send(result);
                repaint.request_repaint();
            })
            .map_err(|_| CatalogError::Service.to_string())?;
        self.danbooru.verification = None;
        self.danbooru.error = None;
        self.danbooru.pending = Some(PendingVerification {
            generation: self.danbooru.generation,
            receiver,
        });
        Ok(())
    }

    pub fn poll_danbooru_verification(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.danbooru.pending else {
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
        let pending = self.danbooru.pending.take().expect("pending verification");
        if pending.generation != self.danbooru.generation || self.demo {
            return;
        }
        self.danbooru.verification = Some(result);
        self.log(
            "INFO",
            if result.is_ok() {
                "Danbooru 账号认证已通过。"
            } else {
                "Danbooru 账号验证失败，请检查设置中的原因。"
            },
        );
    }

    fn invalidate_danbooru_verification(&mut self) {
        self.danbooru.generation = self.danbooru.generation.wrapping_add(1);
        self.danbooru.verification = None;
        self.cancel_tag_lookup();
    }

    pub fn load_danbooru_settings(&mut self) {
        self.invalidate_danbooru_verification();
        match self.danbooru.store.load() {
            Ok(Some(credentials)) => {
                if bdm_application::validate_source_credentials(
                    &credentials.username,
                    &credentials.api_key,
                )
                .is_err()
                {
                    self.danbooru.error = Some("已保存的凭据格式无效，请重新填写。".into());
                    return;
                }
                self.prefs.danbooru_username = credentials.username.clone();
                self.danbooru.username = credentials.username.clone();
                self.danbooru.saved_username = Some(credentials.username);
                self.danbooru.error = None;
            }
            Ok(None) => {
                self.danbooru.saved_username = None;
                self.danbooru.error = None;
            }
            Err(error) => self.danbooru.error = Some(error.to_string()),
        }
    }

    pub fn save_danbooru_settings(&mut self) -> Result<(), String> {
        let username = self.danbooru.username.trim().to_owned();
        if username.is_empty() {
            return Err("请填写 Danbooru 用户名。".into());
        }
        let api_key = if self.danbooru.api_key.trim().is_empty() {
            match self
                .danbooru
                .store
                .load()
                .map_err(|error| error.to_string())?
            {
                Some(credentials) if credentials.username == username => credentials.api_key,
                Some(_) => return Err("更换用户名时请填写新的 API Key。".into()),
                None => return Err("请填写 API Key。".into()),
            }
        } else {
            self.danbooru.api_key.trim().to_owned()
        };
        bdm_application::validate_source_credentials(&username, &api_key).map_err(str::to_owned)?;
        self.danbooru
            .store
            .save(&SourceCredentials {
                username: username.clone(),
                api_key,
            })
            .map_err(|error| error.to_string())?;
        // Update ordinary settings only after the single OS credential write succeeds.
        self.prefs.danbooru_username = username.clone();
        self.danbooru.username = username.clone();
        self.danbooru.saved_username = Some(username);
        self.danbooru.api_key.clear();
        self.danbooru.error = None;
        self.invalidate_danbooru_verification();
        self.log("INFO", "Danbooru 凭据已保存到系统凭据存储，尚未验证连接。");
        Ok(())
    }

    pub fn clear_danbooru_settings(&mut self) -> Result<(), String> {
        self.danbooru
            .store
            .clear()
            .map_err(|error| error.to_string())?;
        self.prefs.danbooru_username.clear();
        self.danbooru.username.clear();
        self.danbooru.api_key.clear();
        self.danbooru.saved_username = None;
        self.danbooru.error = None;
        self.invalidate_danbooru_verification();
        self.log("INFO", "Danbooru 凭据已移除。");
        Ok(())
    }

    pub fn danbooru_settings_card(&mut self, ui: &mut egui::Ui) {
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                section(ui, "Danbooru", "");
                let saved = self.danbooru.saved_username.as_deref()
                    == Some(self.danbooru.username.trim())
                    && self.danbooru.api_key.is_empty();
                badge(
                    ui,
                    if saved {
                        match self.danbooru.verification {
                            Some(Ok(())) => "认证通过",
                            Some(Err(_)) => "验证失败",
                            None if self.danbooru.pending.is_some() => "正在验证",
                            None => "已保存 · 未验证",
                        }
                    } else if self.danbooru.saved_username.is_some()
                        || !self.danbooru.api_key.is_empty()
                        || !self.danbooru.username.trim().is_empty()
                    {
                        "未保存"
                    } else {
                        "未配置"
                    },
                    if saved
                        && self
                            .danbooru
                            .verification
                            .is_some_and(|result| result.is_err())
                    {
                        AMBER
                    } else if saved {
                        ACCENT
                    } else {
                        MUTED
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.hyperlink_to("获取 API Key ↗", "https://danbooru.donmai.us/profile");
                });
            });
            ui.add_space(14.0);
            ui.columns(2, |columns| {
                let username_width = columns[0].available_width();
                let key_width = columns[1].available_width();
                columns[0].label("用户名");
                columns[0].add_sized(
                    [username_width, 36.0],
                    egui::TextEdit::singleline(&mut self.danbooru.username)
                        .hint_text("Danbooru 用户名")
                        .margin(vec2(10.0, 9.0)),
                );
                columns[1].label("API Key");
                let hint = if self.danbooru.saved_username.is_some() {
                    "已保存，留空保留当前密钥"
                } else {
                    "粘贴 API Key"
                };
                columns[1].add_sized(
                    [key_width, 36.0],
                    egui::TextEdit::singleline(&mut self.danbooru.api_key)
                        .password(true)
                        .hint_text(hint)
                        .margin(vec2(10.0, 9.0)),
                );
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if primary(ui, "保存")
                    .on_hover_text("密钥存入 Windows 凭据管理器；保存不会发起网络请求。")
                    .clicked()
                {
                    match self.save_danbooru_settings() {
                        Ok(()) => self.notify(ui.ctx(), "Danbooru 配置已保存"),
                        Err(error) => self.danbooru.error = Some(error),
                    }
                }
                if self.danbooru.saved_username.is_some()
                    && icon_button(ui, "trash", "移除 Danbooru 凭据", false).clicked()
                {
                    match self.clear_danbooru_settings() {
                        Ok(()) => self.notify(ui.ctx(), "Danbooru 凭据已移除"),
                        Err(error) => self.danbooru.error = Some(error),
                    }
                }
                let saved = self.danbooru.saved_username.as_deref()
                    == Some(self.danbooru.username.trim())
                    && self.danbooru.api_key.is_empty();
                if ui
                    .add_enabled(
                        saved
                            && self.danbooru.pending.is_none()
                            && !self.demo
                            && !self.is_capture(),
                        egui::Button::new("验证连接"),
                    )
                    .clicked()
                {
                    if let Err(error) = self.start_danbooru_verification(ui.ctx()) {
                        self.danbooru.error = Some(error);
                    }
                }
                ui.label(RichText::new("danbooru.donmai.us").size(12.0).color(MUTED))
                    .on_hover_text(
                        "验证连接检查已保存账号；在画师／角色页选择收藏后下载静态原图。",
                    );
            });
            if let Some(error) = &self.danbooru.error {
                ui.add(egui::Label::new(RichText::new(error).color(DANGER)).wrap());
            }
            if let Some(Err(error)) = self.danbooru.verification {
                ui.add(egui::Label::new(RichText::new(error.to_string()).color(AMBER)).wrap());
            }
        });
    }

    pub fn danbooru_recovery_form(&mut self, ui: &mut egui::Ui) {
        ui.label("Danbooru 账号");
        ui.add(
            egui::Label::new(
                RichText::new("填写网站用户名和 API Key；不要填写邮箱或登录密码。")
                    .color(MUTED)
                    .size(12.0),
            )
            .wrap(),
        );
        ui.add_space(6.0);
        ui.columns(2, |columns| {
            columns[0].label("用户名");
            columns[0].add(
                egui::TextEdit::singleline(&mut self.danbooru.username)
                    .desired_width(f32::INFINITY),
            );
            columns[1].label("API Key");
            columns[1].add(
                egui::TextEdit::singleline(&mut self.danbooru.api_key)
                    .password(true)
                    .hint_text("粘贴新的 API Key")
                    .desired_width(f32::INFINITY),
            );
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled_ui(
                    !self.tag_lookup_pending() && !self.demo && !self.is_capture(),
                    |ui| primary(ui, "保存并重试"),
                )
                .inner
                .clicked()
            {
                let result = self.save_danbooru_settings().and_then(|()| {
                    self.catalog.anonymous = false;
                    self.start_tag_lookup(ui.ctx())
                });
                match result {
                    Ok(()) => self.catalog.edit_credentials = false,
                    Err(error) => self.danbooru.error = Some(error),
                }
            }
            ui.hyperlink_to("获取 API Key ↗", "https://danbooru.donmai.us/profile");
        });
        if let Some(error) = &self.danbooru.error {
            ui.add(egui::Label::new(RichText::new(error).color(DANGER)).wrap());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Preferences;
    use bdm_ports::CredentialError;
    use eframe::App;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Default, Clone)]
    struct FakeStore {
        value: Rc<RefCell<Option<SourceCredentials>>>,
        failure: Rc<RefCell<bool>>,
    }
    impl SourceCredentialStore for FakeStore {
        fn load(&self) -> Result<Option<SourceCredentials>, CredentialError> {
            if *self.failure.borrow() {
                return Err(CredentialError::Access);
            }
            Ok(self.value.borrow().as_ref().map(|value| SourceCredentials {
                username: value.username.clone(),
                api_key: value.api_key.clone(),
            }))
        }
        fn save(&self, credentials: &SourceCredentials) -> Result<(), CredentialError> {
            if *self.failure.borrow() {
                return Err(CredentialError::Access);
            }
            *self.value.borrow_mut() = Some(SourceCredentials {
                username: credentials.username.clone(),
                api_key: credentials.api_key.clone(),
            });
            Ok(())
        }
        fn clear(&self) -> Result<(), CredentialError> {
            if *self.failure.borrow() {
                return Err(CredentialError::Access);
            }
            *self.value.borrow_mut() = None;
            Ok(())
        }
    }
    fn app() -> (BdmApp, FakeStore) {
        let mut app = BdmApp::from_preferences(Preferences::default(), false, None);
        let store = FakeStore::default();
        app.danbooru.store = Box::new(store.clone());
        app.danbooru.username = "synthetic_user".into();
        app.danbooru.api_key = "synthetic-secret-never-persist".into();
        (app, store)
    }

    #[test]
    fn secure_save_reload_update_and_remove_do_not_persist_or_log_keys() {
        let (mut app, store) = app();
        app.save_danbooru_settings().unwrap();
        assert!(app.danbooru.api_key.is_empty());
        assert_eq!(app.prefs.danbooru_username, "synthetic_user");
        app.load_danbooru_settings();
        assert!(app.danbooru.api_key.is_empty());
        assert_eq!(
            app.danbooru.saved_username.as_deref(),
            Some("synthetic_user")
        );
        // Blank input retains the same account's saved key.
        app.save_danbooru_settings().unwrap();
        assert!(store.load().unwrap().is_some());
        #[derive(Default)]
        struct Storage(Vec<String>);
        impl eframe::Storage for Storage {
            fn get_string(&self, _: &str) -> Option<String> {
                None
            }
            fn set_string(&mut self, _: &str, value: String) {
                self.0.push(value);
            }
            fn flush(&mut self) {}
        }
        // Even an unsaved replacement key must not appear in eframe preferences.
        app.danbooru.api_key = "replacement-secret-never-persist".into();
        let mut storage = Storage::default();
        app.save(&mut storage);
        assert!(storage
            .0
            .iter()
            .all(|value| !value.contains("secret-never-persist")));
        assert!(app
            .logs
            .iter()
            .all(|entry| !entry.message.contains("secret-never-persist")));
        app.clear_danbooru_settings().unwrap();
        assert!(store.load().unwrap().is_none());
        assert!(app.prefs.danbooru_username.is_empty());
        assert!(app.danbooru.api_key.is_empty());
    }

    #[test]
    fn failed_save_and_clear_preserve_previous_identity_and_key() {
        let (mut app, store) = app();
        app.save_danbooru_settings().unwrap();
        app.danbooru.username = "another_user".into();
        assert!(app.save_danbooru_settings().is_err());
        app.danbooru.api_key = "replacement-key".into();
        *store.failure.borrow_mut() = true;
        assert!(app.save_danbooru_settings().is_err());
        assert!(app.clear_danbooru_settings().is_err());
        assert_eq!(app.prefs.danbooru_username, "synthetic_user");
        *store.failure.borrow_mut() = false;
        assert_eq!(store.load().unwrap().unwrap().username, "synthetic_user");
    }

    #[test]
    fn credential_changes_discard_late_verification_and_tag_results() {
        let (mut app, store) = app();
        app.save_danbooru_settings().unwrap();
        let (sender, receiver) = mpsc::channel();
        app.danbooru.pending = Some(PendingVerification {
            generation: app.danbooru.generation,
            receiver,
        });
        app.danbooru.api_key = "replacement-key".into();
        app.save_danbooru_settings().unwrap();
        sender.send(Ok(())).unwrap();
        app.poll_danbooru_verification(&egui::Context::default());
        assert!(app.danbooru.verification.is_none());
        assert!(app.danbooru.pending.is_none());
        let (sender, receiver) = mpsc::channel();
        app.danbooru.pending = Some(PendingVerification {
            generation: app.danbooru.generation,
            receiver,
        });
        app.clear_danbooru_settings().unwrap();
        sender.send(Err(CatalogError::Authentication)).unwrap();
        app.poll_danbooru_verification(&egui::Context::default());
        assert!(app.danbooru.verification.is_none());
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn verification_uses_saved_credentials_and_surfaces_failures_without_keys() {
        let (mut app, store) = app();
        let ctx = egui::Context::default();
        assert!(app.start_danbooru_verification(&ctx).is_err());
        assert!(app.danbooru.pending.is_none());
        app.save_danbooru_settings().unwrap();
        app.danbooru.api_key = "unsaved-replacement".into();
        assert_eq!(
            app.saved_danbooru_credentials().unwrap().unwrap().api_key,
            "synthetic-secret-never-persist"
        );
        let (sender, receiver) = mpsc::channel();
        app.danbooru.pending = Some(PendingVerification {
            generation: app.danbooru.generation,
            receiver,
        });
        sender.send(Err(CatalogError::Authentication)).unwrap();
        app.poll_danbooru_verification(&ctx);
        assert_eq!(
            app.danbooru.verification,
            Some(Err(CatalogError::Authentication))
        );
        assert!(!app
            .logs
            .iter()
            .any(|entry| entry.message.contains("synthetic-secret")
                || entry.message.contains("unsaved-replacement")));
        *store.failure.borrow_mut() = true;
        assert!(app.start_danbooru_verification(&ctx).is_err());
        app.demo = true;
        assert!(app.start_danbooru_verification(&ctx).is_err());
        app.demo = false;
        app.danbooru.verification = Some(Ok(()));
        assert!(app.save_danbooru_settings().is_err());
        assert_eq!(app.danbooru.verification, Some(Ok(())));
    }
}
