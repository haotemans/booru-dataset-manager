#![forbid(unsafe_code)]
use serde::{Deserialize, Serialize};

/// Hard provider-side safety limits shared by the desktop settings and uploader.
pub const AI_IMAGE_MAX_EDGE: u32 = 1024;
pub const AI_IMAGE_MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum JsonOutputMode {
    /// Strict JSON is requested in the prompt and validated locally.
    PromptOnly,
    #[default]
    JsonObject,
    JsonSchema,
}

impl JsonOutputMode {
    pub fn description(self) -> &'static str {
        match self {
            Self::PromptOnly => "提示词 JSON（兼容模式）",
            Self::JsonObject => "JSON Object",
            Self::JsonSchema => "JSON Schema（需服务支持）",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AiRequestSettings {
    pub batch_size: usize,
    pub timeout_seconds: u64,
    pub transport_retries: u8,
    pub json_output: JsonOutputMode,
    pub send_images: bool,
    pub image_max_edge: u32,
    pub jpeg_quality: u8,
}

impl Default for AiRequestSettings {
    fn default() -> Self {
        Self {
            batch_size: 8,
            timeout_seconds: 120,
            transport_retries: 1,
            json_output: JsonOutputMode::default(),
            send_images: false,
            image_max_edge: AI_IMAGE_MAX_EDGE,
            jpeg_quality: 85,
        }
    }
}

impl AiRequestSettings {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=8).contains(&self.batch_size) {
            return Err("每批数量必须在 1–8 之间。");
        }
        if !(10..=600).contains(&self.timeout_seconds) {
            return Err("请求超时必须在 10–600 秒之间。");
        }
        if self.transport_retries > 3 {
            return Err("网络重试次数必须在 0–3 之间。");
        }
        if !(64..=AI_IMAGE_MAX_EDGE).contains(&self.image_max_edge) {
            return Err("图片最长边必须在 64–1024 像素之间。");
        }
        if !(40..=95).contains(&self.jpeg_quality) {
            return Err("JPEG 质量必须在 40–95 之间。");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    pub library_root: Option<String>,
    pub minimum_width: u32,
    pub minimum_height: u32,
    pub allow_unknown_dimensions: bool,
    #[serde(default = "default_flatten_transparency")]
    pub flatten_transparency: bool,
    #[serde(default)]
    pub download_media: DownloadMediaPolicy,
}

fn default_flatten_transparency() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum DownloadMediaPolicy {
    #[default]
    StaticImagesOnly,
}

impl DownloadMediaPolicy {
    pub fn description(self) -> &'static str {
        match self {
            Self::StaticImagesOnly => "仅静态图片 · 跳过 GIF、视频及其他动画",
        }
    }
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            library_root: None,
            minimum_width: 800,
            minimum_height: 800,
            allow_unknown_dimensions: false,
            flatten_transparency: true,
            download_media: DownloadMediaPolicy::default(),
        }
    }
}
