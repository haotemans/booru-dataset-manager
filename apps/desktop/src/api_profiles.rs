use crate::{
    app::{BdmApp, Dialog},
    model::{new_profile_id, ApiProfile},
};
use bdm_ports::{ApiCredentialStore, ApiCredentials};

pub struct ApiProfileSettings {
    pub key_draft: String,
    pub key_saved: bool,
    pub load_error: Option<String>,
    pub store: Box<dyn ApiCredentialStore>,
}

impl Default for ApiProfileSettings {
    fn default() -> Self {
        Self {
            key_draft: String::new(),
            key_saved: false,
            load_error: None,
            store: Box::new(bdm_infrastructure::credentials::NativeApiCredentialStore),
        }
    }
}

impl BdmApp {
    pub fn begin_api_profile(&mut self, index: Option<usize>) {
        self.api_profiles.key_draft.clear();
        self.api_profiles.key_saved = false;
        self.api_profiles.load_error = None;
        self.form_error.clear();
        self.profile_draft = index
            .and_then(|i| self.prefs.profiles.get(i))
            .cloned()
            .unwrap_or_else(|| ApiProfile {
                id: new_profile_id(),
                concurrency: 2,
                ..Default::default()
            });
        if index.is_some() {
            match self.api_profiles.store.load(&self.profile_draft.id) {
                Ok(value) => self.api_profiles.key_saved = value.is_some(),
                Err(error) => self.api_profiles.load_error = Some(error.to_string()),
            }
        }
        self.dialog = Some(Dialog::Profile(index));
    }

    pub fn save_api_profile(&mut self, index: Option<usize>) -> Result<(), String> {
        self.profile_draft.validate().map_err(str::to_owned)?;
        if let Some(i) = index {
            if self.prefs.profiles.get(i).map(|p| &p.id) != Some(&self.profile_draft.id) {
                return Err("配置已变化，请重新打开编辑。".into());
            }
        } else if self
            .prefs
            .profiles
            .iter()
            .any(|p| p.id == self.profile_draft.id)
        {
            return Err("配置标识重复，请重新添加。".into());
        }
        let endpoint = url::Url::parse(self.profile_draft.endpoint.trim())
            .map_err(|_| "服务地址无效。".to_owned())?
            .to_string();
        let endpoint = endpoint.trim_end_matches('/').to_owned();
        let key = self.api_profiles.key_draft.trim();
        if !key.is_empty() {
            if key.len() > 1024
                || endpoint.len() + key.len() > 1279
                || key.chars().any(|c| c.is_whitespace() || c.is_control())
            {
                return Err("API Key 格式无效或过长，请检查复制内容。".into());
            }
            self.api_profiles
                .store
                .save(
                    &self.profile_draft.id,
                    &ApiCredentials {
                        endpoint,
                        api_key: key.to_owned(),
                    },
                )
                .map_err(|error| error.to_string())?;
            self.api_profiles.key_saved = true;
        } else if index.is_some() {
            let stored = self
                .api_profiles
                .store
                .load(&self.profile_draft.id)
                .map_err(|error| error.to_string())?;
            if stored
                .as_ref()
                .is_some_and(|value| value.endpoint != endpoint)
            {
                return Err("更换服务地址时请填写新密钥，或先移除旧密钥。".into());
            }
            self.api_profiles.key_saved = stored.is_some();
        } else {
            self.api_profiles.key_saved = false;
        }
        let mut draft = self.profile_draft.clone();
        draft.name = draft.name.trim().to_owned();
        draft.endpoint = draft.endpoint.trim().to_owned();
        draft.model = draft.model.trim().to_owned();
        draft.group = draft.group.trim().to_owned();
        if let Some(i) = index {
            self.prefs.profiles[i] = draft;
        } else {
            self.prefs.profiles.push(draft);
        }
        self.api_profiles.key_draft.clear();
        self.api_profiles.load_error = None;
        self.log(
            "INFO",
            "API 配置已保存，连接未验证；密钥仅存于系统凭据存储。",
        );
        Ok(())
    }

    pub fn clear_api_profile_key(&mut self) -> Result<(), String> {
        self.api_profiles
            .store
            .clear(&self.profile_draft.id)
            .map_err(|error| error.to_string())?;
        self.api_profiles.key_draft.clear();
        self.api_profiles.key_saved = false;
        self.api_profiles.load_error = None;
        self.log("INFO", "已移除当前 API 配置的密钥。");
        Ok(())
    }

    pub fn remove_api_profile(&mut self, index: usize) -> Result<(), String> {
        let profile = self.prefs.profiles.get(index).ok_or("配置已不存在。")?;
        self.api_profiles
            .store
            .clear(&profile.id)
            .map_err(|error| error.to_string())?;
        self.prefs.profiles.remove(index);
        self.log("INFO", "已移除 API 配置及其密钥。");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Preferences;
    use bdm_ports::CredentialError;
    use eframe::{egui, App};
    use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

    #[derive(Clone, Default)]
    struct FakeStore {
        values: Rc<RefCell<BTreeMap<String, (String, String)>>>,
        fail: Rc<RefCell<bool>>,
    }
    impl ApiCredentialStore for FakeStore {
        fn load(&self, id: &str) -> Result<Option<ApiCredentials>, CredentialError> {
            if *self.fail.borrow() {
                return Err(CredentialError::Access);
            }
            Ok(self
                .values
                .borrow()
                .get(id)
                .map(|(endpoint, key)| ApiCredentials {
                    endpoint: endpoint.clone(),
                    api_key: key.clone(),
                }))
        }
        fn save(&self, id: &str, value: &ApiCredentials) -> Result<(), CredentialError> {
            if *self.fail.borrow() {
                return Err(CredentialError::Access);
            }
            self.values
                .borrow_mut()
                .insert(id.into(), (value.endpoint.clone(), value.api_key.clone()));
            Ok(())
        }
        fn clear(&self, id: &str) -> Result<(), CredentialError> {
            if *self.fail.borrow() {
                return Err(CredentialError::Access);
            }
            self.values.borrow_mut().remove(id);
            Ok(())
        }
    }
    fn app() -> (BdmApp, FakeStore) {
        let mut app = BdmApp::from_preferences(Preferences::default(), false, None);
        let store = FakeStore::default();
        app.api_profiles.store = Box::new(store.clone());
        (app, store)
    }
    fn draft(app: &mut BdmApp, key: &str) {
        app.begin_api_profile(None);
        app.profile_draft.name = "Same name".into();
        app.profile_draft.endpoint = "https://example.invalid/v1/".into();
        app.profile_draft.model = "model".into();
        app.api_profiles.key_draft = key.into();
    }

    #[test]
    fn independent_profile_keys_survive_rename_and_index_changes() {
        let (mut app, store) = app();
        draft(&mut app, "synthetic-key-a");
        app.save_api_profile(None).unwrap();
        let a = app.prefs.profiles[0].id.clone();
        draft(&mut app, "synthetic-key-b");
        app.save_api_profile(None).unwrap();
        let b = app.prefs.profiles[1].id.clone();
        assert_ne!(a, b);
        app.begin_api_profile(Some(0));
        assert!(app.api_profiles.key_saved);
        assert!(app.api_profiles.key_draft.is_empty());
        app.profile_draft.name = "Renamed".into();
        app.save_api_profile(Some(0)).unwrap();
        assert_eq!(store.load(&a).unwrap().unwrap().api_key, "synthetic-key-a");
        app.remove_api_profile(0).unwrap();
        assert!(store.load(&a).unwrap().is_none());
        assert_eq!(app.prefs.profiles[0].id, b);
        assert_eq!(store.load(&b).unwrap().unwrap().api_key, "synthetic-key-b");
        app.begin_api_profile(Some(0));
        app.clear_api_profile_key().unwrap();
        assert!(store.load(&b).unwrap().is_none());
        assert!(!app.api_profiles.key_saved);
        app.save_api_profile(Some(0)).unwrap();
        assert!(store.load(&b).unwrap().is_none());
    }

    #[test]
    fn endpoint_changes_need_new_keys_and_failed_writes_preserve_previous_profile() {
        let (mut app, store) = app();
        draft(&mut app, "synthetic-old");
        app.save_api_profile(None).unwrap();
        let id = app.prefs.profiles[0].id.clone();
        app.begin_api_profile(Some(0));
        app.profile_draft.endpoint = "https://other.invalid/v1".into();
        assert!(app.save_api_profile(Some(0)).is_err());
        assert_eq!(
            app.prefs.profiles[0].endpoint,
            "https://example.invalid/v1/"
        );
        app.api_profiles.key_draft = "synthetic-new".into();
        *store.fail.borrow_mut() = true;
        assert!(app.save_api_profile(Some(0)).is_err());
        assert_eq!(
            app.prefs.profiles[0].endpoint,
            "https://example.invalid/v1/"
        );
        assert!(app.clear_api_profile_key().is_err());
        assert!(app.api_profiles.key_saved);
        assert!(app.remove_api_profile(0).is_err());
        assert_eq!(app.prefs.profiles.len(), 1);
        *store.fail.borrow_mut() = false;
        assert_eq!(store.load(&id).unwrap().unwrap().api_key, "synthetic-old");
        app.save_api_profile(Some(0)).unwrap();
        let saved = store.load(&id).unwrap().unwrap();
        assert_eq!(saved.endpoint, "https://other.invalid/v1");
        assert_eq!(saved.api_key, "synthetic-new");
    }

    #[test]
    fn keys_never_reach_preferences_logs_or_cancelled_forms() {
        let (mut app, store) = app();
        draft(&mut app, "synthetic-saved-secret");
        app.save_api_profile(None).unwrap();
        let id = app.prefs.profiles[0].id.clone();
        app.begin_api_profile(Some(0));
        app.api_profiles.key_draft = "synthetic-unsaved-secret".into();
        #[derive(Default)]
        struct Storage(String);
        impl eframe::Storage for Storage {
            fn get_string(&self, _: &str) -> Option<String> {
                None
            }
            fn set_string(&mut self, _: &str, value: String) {
                self.0 = value;
            }
            fn flush(&mut self) {}
        }
        let mut storage = Storage::default();
        app.save(&mut storage);
        for secret in ["synthetic-saved-secret", "synthetic-unsaved-secret"] {
            assert!(!storage.0.contains(secret));
            assert!(!app.logs.iter().any(|e| e.message.contains(secret)));
        }
        let ctx = egui::Context::default();
        crate::theme::setup(&ctx);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1050.0, 720.0),
                )),
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
        assert!(app.dialog.is_none());
        assert!(app.api_profiles.key_draft.is_empty());
        assert_eq!(
            store.load(&id).unwrap().unwrap().api_key,
            "synthetic-saved-secret"
        );
    }

    #[test]
    fn invalid_keys_and_metadata_cannot_overwrite_saved_values() {
        let (mut app, store) = app();
        draft(&mut app, "synthetic-valid");
        app.save_api_profile(None).unwrap();
        let id = app.prefs.profiles[0].id.clone();
        app.begin_api_profile(Some(0));
        for key in ["bad key".to_owned(), "bad\0key".into(), "a".repeat(1025)] {
            app.api_profiles.key_draft = key;
            assert!(app.save_api_profile(Some(0)).is_err());
        }
        app.api_profiles.key_draft = "synthetic-new".into();
        app.profile_draft.endpoint = "https://user:secret@example.invalid".into();
        assert!(app.save_api_profile(Some(0)).is_err());
        assert_eq!(store.load(&id).unwrap().unwrap().api_key, "synthetic-valid");
        assert_eq!(
            app.prefs.profiles[0].endpoint,
            "https://example.invalid/v1/"
        );
    }

    #[test]
    fn no_auth_profiles_can_be_saved_without_accessing_credential_storage() {
        let (mut app, store) = app();
        draft(&mut app, "");
        *store.fail.borrow_mut() = true;
        app.save_api_profile(None).unwrap();
        assert_eq!(app.prefs.profiles.len(), 1);
        assert!(!app.api_profiles.key_saved);
    }
}
