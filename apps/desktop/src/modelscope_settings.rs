use crate::{
    app::BdmApp,
    paths,
    theme::{card, MUTED},
    widgets::setting_header,
};
use bdm_config::ModelScopeConfig;
use bdm_infrastructure::{credentials::NativeCredentialStore, modelscope};
use bdm_ports::{SourceCredentialStore, SourceCredentials};
use eframe::egui::{self, RichText};
use std::path::Path;

#[derive(Default)]
pub struct ModelScopeSettings {
    dataset: String,
    token: String,
    saved_dataset: String,
    token_saved: bool,
    error: Option<String>,
}

impl ModelScopeSettings {
    pub fn load(&mut self, root: &Path) {
        let result = (|| -> Result<(), String> {
            if let Some(config) = modelscope::load_config(root).map_err(|e| e.to_string())? {
                self.dataset.clone_from(&config.dataset_id);
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
        let config = ModelScopeConfig::new(&self.dataset)?;
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
        Ok(())
    }

    fn clear_token(&mut self) -> Result<(), String> {
        NativeCredentialStore::modelscope(&self.saved_dataset)
            .clear()
            .map_err(|e| e.to_string())?;
        self.token.clear();
        self.token_saved = false;
        self.error = None;
        Ok(())
    }
}

impl BdmApp {
    pub fn modelscope_settings_card(&mut self, ui: &mut egui::Ui) {
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            setting_header(ui, "lock", "魔搭备份");
            ui.add_space(8.0);
            let settings = &mut self.modelscope;
            let field_width = ui.available_width().min(360.0);
            ui.label("私有数据集 ID");
            ui.add(
                egui::TextEdit::singleline(&mut settings.dataset)
                    .hint_text("用户名/数据集名")
                    .desired_width(field_width),
            );
            ui.label("Access Token");
            let has_token =
                settings.token_saved && settings.dataset.trim() == settings.saved_dataset;
            ui.add(
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
                    .add_enabled(!self.demo, egui::Button::new("保存魔搭设置"))
                    .clicked()
                {
                    match self.modelscope.save(&paths::project_root()) {
                        Ok(()) => self.notify(ui.ctx(), "魔搭设置已保存。"),
                        Err(error) => self.modelscope.error = Some(error),
                    }
                }
                if ui
                    .add_enabled(!self.demo && has_token, egui::Button::new("移除令牌"))
                    .clicked()
                {
                    match self.modelscope.clear_token() {
                        Ok(()) => self.notify(ui.ctx(), "已移除本机保存的魔搭令牌。"),
                        Err(error) => self.modelscope.error = Some(error),
                    }
                }
            });
            if let Some(error) = &self.modelscope.error {
                ui.label(RichText::new(error).color(MUTED));
            }
        });
    }
}
