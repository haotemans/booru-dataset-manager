use crate::{app::BdmApp, theme::*, widgets::*};
use bdm_domain::sources::DownloadSource;
use bdm_ports::{CatalogError, SourceCredentialStore, SourceCredentials};
use eframe::egui::{self, vec2, RichText};
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub struct GelbooruSettings {
    pub user_id: String,
    pub api_key: String,
    pub saved_user_id: Option<String>,
    pub error: Option<String>,
    pub store: Box<dyn SourceCredentialStore>,
    pub verification: Option<Result<(), CatalogError>>,
    generation: u64,
    pending: Option<(u64, Receiver<Result<(), CatalogError>>)>,
}

impl GelbooruSettings {
    pub fn new(user_id: String) -> Self {
        let root = crate::paths::project_root();
        let portable = root.join("portable.flag").is_file();
        Self {
            user_id,
            api_key: String::new(),
            saved_user_id: None,
            error: None,
            store: Box::new(
                bdm_infrastructure::credentials::NativeCredentialStore::gelbooru(
                    portable.then_some(root.as_path()),
                ),
            ),
            verification: None,
            generation: 0,
            pending: None,
        }
    }
}
impl BdmApp {
    pub fn saved_gelbooru_credentials(&self) -> Result<Option<SourceCredentials>, CatalogError> {
        if let Some(id) = &self.gelbooru.saved_user_id {
            let credentials = self
                .gelbooru
                .store
                .load()
                .map_err(|_| CatalogError::GelbooruCredentials)?
                .ok_or(CatalogError::GelbooruCredentials)?;
            if &credentials.username != id
                || !bdm_infrastructure::gelbooru::valid_credentials(&credentials)
            {
                return Err(CatalogError::GelbooruCredentials);
            }
            Ok(Some(credentials))
        } else if self.gelbooru.error.is_some() && !self.prefs.gelbooru_user_id.is_empty() {
            Err(CatalogError::GelbooruCredentials)
        } else {
            Ok(None)
        }
    }
    pub fn load_gelbooru_settings(&mut self) {
        match self.gelbooru.store.load() {
            Ok(Some(value)) if bdm_infrastructure::gelbooru::valid_credentials(&value) => {
                self.prefs.gelbooru_user_id = value.username.clone();
                self.gelbooru.user_id = value.username.clone();
                self.gelbooru.saved_user_id = Some(value.username);
                self.gelbooru.error = None;
            }
            Ok(None) => {
                self.gelbooru.saved_user_id = None;
                self.gelbooru.error = None;
            }
            Ok(Some(_)) => {
                self.gelbooru.error = Some("已保存的 Gelbooru 凭据格式无效，请重新保存。".into())
            }
            Err(error) => self.gelbooru.error = Some(error.to_string()),
        }
    }
    pub fn save_gelbooru_settings(&mut self) -> Result<(), String> {
        if self.demo || self.is_capture() {
            return Err("示例模式不修改来源凭据。".into());
        }
        let id = self
            .gelbooru
            .user_id
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or("User ID 必须是大于 0 的数字。")?
            .to_string();
        let key = if self.gelbooru.api_key.trim().is_empty() {
            let old = self
                .saved_gelbooru_credentials()
                .map_err(|e| e.to_string())?
                .ok_or("请填写 Gelbooru API Key。")?;
            if old.username != id {
                return Err("更换 User ID 时必须重新填写 API Key。".into());
            }
            old.api_key
        } else {
            self.gelbooru.api_key.trim().to_owned()
        };
        let credentials = SourceCredentials {
            username: id.clone(),
            api_key: key,
        };
        if !bdm_infrastructure::gelbooru::valid_credentials(&credentials) {
            return Err("Gelbooru API Key 格式无效。".into());
        }
        self.gelbooru
            .store
            .save(&credentials)
            .map_err(|e| e.to_string())?;
        self.gelbooru.saved_user_id = Some(id.clone());
        self.gelbooru.user_id = id.clone();
        self.prefs.gelbooru_user_id = id;
        self.gelbooru.api_key.clear();
        self.gelbooru.error = None;
        self.gelbooru.verification = None;
        self.gelbooru.generation = self.gelbooru.generation.wrapping_add(1);
        self.cancel_tag_lookup();
        Ok(())
    }
    fn clear_gelbooru_settings(&mut self) -> Result<(), String> {
        if self.demo || self.is_capture() {
            return Err("示例模式不修改来源凭据。".into());
        }
        self.gelbooru.store.clear().map_err(|e| e.to_string())?;
        self.gelbooru.saved_user_id = None;
        self.gelbooru.user_id.clear();
        self.gelbooru.api_key.clear();
        self.prefs.gelbooru_user_id.clear();
        self.gelbooru.verification = None;
        self.gelbooru.error = None;
        self.gelbooru.generation = self.gelbooru.generation.wrapping_add(1);
        self.cancel_tag_lookup();
        Ok(())
    }
    fn start_gelbooru_verification(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if self.demo || self.is_capture() {
            return Err("示例模式不请求远程 API。".into());
        }
        let credentials = self
            .saved_gelbooru_credentials()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| CatalogError::GelbooruCredentials.to_string())?;
        if self.gelbooru.pending.is_some() {
            return Err("正在验证，请稍后。".into());
        }
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name("gelbooru-api-verification".into())
            .spawn(move || {
                let result = bdm_infrastructure::gelbooru::GelbooruProvider::new(Some(credentials))
                    .and_then(|p| p.verify_credentials());
                let _ = sender.send(result);
                repaint.request_repaint();
            })
            .map_err(|_| "无法启动验证任务。".to_owned())?;
        self.gelbooru.verification = None;
        self.gelbooru.error = None;
        self.gelbooru.pending = Some((self.gelbooru.generation, receiver));
        Ok(())
    }
    pub fn poll_gelbooru_verification(&mut self, ctx: &egui::Context) {
        let Some((generation, receiver)) = &self.gelbooru.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Disconnected) => Err(CatalogError::Service),
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
        };
        let valid = *generation == self.gelbooru.generation && !self.demo;
        self.gelbooru.pending = None;
        if valid {
            self.gelbooru.verification = Some(result);
        }
    }
    pub fn gelbooru_settings_card(&mut self, ui: &mut egui::Ui) {
        card().show(ui,|ui| {
            ui.set_width(ui.available_width());section(ui,"图片来源与 Gelbooru 备用","download");ui.add_space(12.0);
            let before=self.prefs.download_source;
            egui::ComboBox::from_id_salt("download_source").width(300.0).selected_text(before.label()).show_ui(ui,|ui| {
                for mode in [DownloadSource::Automatic,DownloadSource::Danbooru,DownloadSource::Gelbooru] {
                    ui.selectable_value(&mut self.prefs.download_source,mode,mode.label());
                }
            });
            if before!=self.prefs.download_source {self.cancel_tag_lookup();}
            ui.add(egui::Label::new(RichText::new("自动模式在连接／服务异常经有限重试后切换备用源；认证失败、网站验证和限流不自动切换。每个任务分别保存两站进度。来源设置用于新任务，续传使用任务已保存的来源。").size(12.0).color(MUTED)).wrap());
            ui.add_space(12.0);
            ui.horizontal(|ui| {ui.label(RichText::new("Gelbooru").strong());ui.hyperlink_to("注册／获取 API 凭据 ↗","https://gelbooru.com/index.php?page=account&s=options");});
            ui.columns(2,|columns| {
                columns[0].label("User ID（数字账号 ID）");
                let width=columns[0].available_width();columns[0].add_sized([width,36.0],egui::TextEdit::singleline(&mut self.gelbooru.user_id).hint_text("例如 123456").margin(vec2(10.0,9.0)));
                columns[1].label("API Key");
                let width=columns[1].available_width();columns[1].add_sized([width,36.0],egui::TextEdit::singleline(&mut self.gelbooru.api_key).password(true).hint_text("已保存时可留空保留").margin(vec2(10.0,9.0)));
            });
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if primary(ui,"保存 Gelbooru 配置").clicked() {if let Err(error)=self.save_gelbooru_settings(){self.gelbooru.error=Some(error)}else{self.notify(ui.ctx(),"Gelbooru 配置已安全保存");}}
                let saved=self.gelbooru.saved_user_id.as_deref()==Some(self.gelbooru.user_id.trim())&&self.gelbooru.api_key.is_empty();
                if ui.add_enabled(saved&&self.gelbooru.pending.is_none()&&!self.demo&&!self.is_capture(),egui::Button::new("验证 API 连接")).clicked() {
                    if let Err(error)=self.start_gelbooru_verification(ui.ctx()){self.gelbooru.error=Some(error)}
                }
                if self.gelbooru.saved_user_id.is_some()&&icon_button(ui,"trash","移除 Gelbooru 凭据",false).clicked() {
                    if let Err(error)=self.clear_gelbooru_settings(){self.gelbooru.error=Some(error)}
                }
                if self.gelbooru.pending.is_some(){ui.spinner();ui.label("验证中…");}
                else if saved {
                    ui.label(match self.gelbooru.verification {Some(Ok(()))=>"API 连接可用",Some(Err(_))=>"API 验证失败",None=>"已保存 · 未验证"});
                }
            });
            if let Some(error)=&self.gelbooru.error {ui.add(egui::Label::new(RichText::new(error).color(AMBER)).wrap());}
            if let Some(Err(error))=&self.gelbooru.verification {ui.add(egui::Label::new(RichText::new(format!("Gelbooru：{error}")).color(AMBER)).wrap());}
            ui.add(egui::Label::new(RichText::new("凭据独立存入 Windows 凭据管理器；保存不会联网，也不会复用 Danbooru 或 AI API 密钥。").size(12.0).color(MUTED)).wrap());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    struct Store(Rc<RefCell<Option<SourceCredentials>>>);
    #[derive(Default)]
    struct Storage(String);
    impl eframe::Storage for Storage {
        fn get_string(&self, _: &str) -> Option<String> {
            Some(self.0.clone())
        }
        fn set_string(&mut self, _: &str, value: String) {
            self.0 = value;
        }
        fn flush(&mut self) {}
    }
    impl SourceCredentialStore for Store {
        fn load(&self) -> Result<Option<SourceCredentials>, bdm_ports::CredentialError> {
            Ok(self.0.borrow().as_ref().map(|v| SourceCredentials {
                username: v.username.clone(),
                api_key: v.api_key.clone(),
            }))
        }
        fn save(&self, value: &SourceCredentials) -> Result<(), bdm_ports::CredentialError> {
            *self.0.borrow_mut() = Some(SourceCredentials {
                username: value.username.clone(),
                api_key: value.api_key.clone(),
            });
            Ok(())
        }
        fn clear(&self) -> Result<(), bdm_ports::CredentialError> {
            *self.0.borrow_mut() = None;
            Ok(())
        }
    }
    #[test]
    fn settings_store_keys_separately_and_blank_keys_cannot_cross_accounts() {
        let mut app = BdmApp::from_preferences(crate::model::Preferences::default(), false, None);
        let saved = Rc::new(RefCell::new(None));
        app.gelbooru.store = Box::new(Store(saved.clone()));
        app.gelbooru.user_id = "123".into();
        app.gelbooru.api_key = "synthetic-gelbooru-key".into();
        app.save_gelbooru_settings().unwrap();
        assert_eq!(app.prefs.gelbooru_user_id, "123");
        assert!(app.gelbooru.api_key.is_empty());
        let mut storage = Storage::default();
        eframe::set_value(&mut storage, eframe::APP_KEY, &app.prefs);
        assert!(storage.0.contains("123"));
        assert!(!storage.0.contains("synthetic-gelbooru-key"));
        app.save_gelbooru_settings().unwrap();
        app.gelbooru.user_id = "456".into();
        assert!(app.save_gelbooru_settings().is_err());
        assert_eq!(saved.borrow().as_ref().unwrap().username, "123");
        app.clear_gelbooru_settings().unwrap();
        assert!(saved.borrow().is_none());
        assert!(app.prefs.gelbooru_user_id.is_empty());
        app.demo = true;
        app.gelbooru.api_key = "synthetic".into();
        assert!(app.save_gelbooru_settings().is_err());
        assert!(saved.borrow().is_none());
    }
}
