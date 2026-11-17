//! Bounded, display-only encoded previews. No library files are written here.
use super::{
    service::{ServiceImage, ServiceReply},
    Settings,
};
use bdm_ports::IngestError;
use image::{DynamicImage, GenericImageView, ImageFormat};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::VecDeque, sync::Arc};

const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 32;

#[derive(Clone, Copy, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PreviewFormat {
    #[default]
    Png,
    Jpeg,
}

#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PreviewOptions {
    /// Longest display edge, 256..2048; default 1024. Never upscale small images.
    #[schemars(range(min = 256, max = 2048))]
    pub max_edge: u16,
    /// PNG is lossless; JPEG quality 90 is for viewing only, never saved crops.
    pub format: PreviewFormat,
    /// Reuse only a preview actually retained/inspected by this client. Omit if
    /// the client lost the image. First full view of a lease always sends pixels.
    pub known_preview_token: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(bytes: Arc<[u8]>) -> Cached {
        Cached {
            image: ServiceImage {
                bytes,
                mime_type: "image/png",
            },
            native: (1, 1),
            display: (1, 1),
            sha256: String::new(),
            token: String::new(),
            facts: Value::Null,
        }
    }
    #[test]
    fn cache_evicts_old_entries_and_stays_within_encoded_memory_budget() {
        let mut cache = Cache::default();
        for i in 0..MAX_ENTRIES {
            cache.insert(i.to_string(), entry(Arc::from([0_u8; 16])));
        }
        assert!(cache.get("0").is_some());
        cache.insert("new".into(), entry(Arc::from([0_u8; 16])));
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(cache.get("1").is_none());
        assert!(cache.get("0").is_some());
        let bytes: Arc<[u8]> = Arc::from(vec![0; 4 * 1024 * 1024]);
        for i in 0..12 {
            cache.insert(format!("large-{i}"), entry(bytes.clone()));
            assert!(cache.bytes <= MAX_BYTES);
            assert!(cache.entries.len() <= MAX_ENTRIES);
        }
        assert!(cache.get("large-0").is_none());
        assert!(cache.get("large-11").is_some());
    }
}
impl Default for PreviewOptions {
    fn default() -> Self {
        Self {
            max_edge: 1024,
            format: PreviewFormat::Png,
            known_preview_token: None,
        }
    }
}
impl PreviewOptions {
    pub fn compact() -> Self {
        Self {
            format: PreviewFormat::Jpeg,
            ..Self::default()
        }
    }
    pub fn validate(&self) -> Result<(), IngestError> {
        if !(256..=2048).contains(&self.max_edge)
            || self
                .known_preview_token
                .as_ref()
                .is_some_and(|t| t.len() != 64 || !t.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(IngestError::StorageDetails(json!({"code":"invalid_preview","message":"max_edge must be 256..2048; known_preview_token must be the exact returned 64-character token"}).to_string()));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Cached {
    image: ServiceImage,
    native: (u32, u32),
    display: (u32, u32),
    sha256: String,
    token: String,
    facts: Value,
}
#[derive(Default)]
pub(super) struct Cache {
    entries: VecDeque<(String, Cached)>,
    bytes: usize,
}
impl Cache {
    fn get(&mut self, key: &str) -> Option<Cached> {
        let index = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(index)?;
        let result = entry.1.clone();
        self.entries.push_back(entry);
        Some(result)
    }
    fn insert(&mut self, key: String, value: Cached) {
        if let Some(index) = self.entries.iter().position(|(k, _)| k == &key) {
            if let Some((_, old)) = self.entries.remove(index) {
                self.bytes -= old.image.bytes.len();
            }
        }
        let len = value.image.bytes.len();
        if len > MAX_BYTES {
            return;
        }
        while self.bytes + len > MAX_BYTES || self.entries.len() >= MAX_ENTRIES {
            if let Some((_, old)) = self.entries.pop_front() {
                self.bytes -= old.image.bytes.len();
            } else {
                break;
            }
        }
        self.bytes += len;
        self.entries.push_back((key, value));
    }
}

pub(super) struct Prepared {
    value: Cached,
    hit: bool,
}
impl Prepared {
    pub fn facts(&self) -> &Value {
        &self.value.facts
    }
    pub fn reply(
        self,
        mut metadata: Value,
        options: &PreviewOptions,
        allow_reuse: bool,
    ) -> ServiceReply {
        let same = allow_reuse && options.known_preview_token.as_deref() == Some(&self.value.token);
        metadata["native_view_dimensions"] = json!(self.value.native);
        metadata["display_dimensions"] = json!(self.value.display);
        metadata["display_preview"] = json!({"mime_type":self.value.image.mime_type,"max_edge":options.max_edge,"byte_length":self.value.image.bytes.len(),"sha256":self.value.sha256,"token":self.value.token,"cache_hit":self.hit,"not_modified":same,"stored_pixels_unchanged":true});
        if same {
            metadata["display_preview"]["hint"]=json!("Reuse this exact previously inspected preview; omit known_preview_token to request pixels again if no longer available.");
        }
        ServiceReply {
            metadata,
            image: if same { None } else { Some(self.value.image) },
            is_error: false,
        }
    }
}

pub(super) fn prepare(
    settings: &Settings,
    source: &str,
    options: &PreviewOptions,
    load: impl FnOnce() -> Result<(DynamicImage, Value), IngestError>,
) -> Result<Prepared, IngestError> {
    options.validate()?;
    let key = format!(
        "{:x}",
        Sha256::digest(
            json!([source, options.max_edge, options.format])
                .to_string()
                .as_bytes()
        )
    );
    if let Some(value) = settings
        .preview_cache
        .lock()
        .map_err(|_| IngestError::Storage)?
        .get(&key)
    {
        return Ok(Prepared { value, hit: true });
    }
    let (image, facts) = load()?;
    let native = image.dimensions();
    let edge = u32::from(options.max_edge);
    let display = if native.0 > edge || native.1 > edge {
        image.thumbnail(edge, edge)
    } else {
        image
    };
    let dimensions = display.dimensions();
    let mut buffer = std::io::Cursor::new(Vec::new());
    let mime_type = match options.format {
        PreviewFormat::Png => {
            display
                .write_to(&mut buffer, ImageFormat::Png)
                .map_err(|_| IngestError::InvalidMedia)?;
            "image/png"
        }
        PreviewFormat::Jpeg => {
            // Keep transparent artwork interpretable on a white display canvas.
            let rgba = display.to_rgba8();
            let rgb = image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
                let p = rgba.get_pixel(x, y).0;
                let a = u16::from(p[3]);
                image::Rgb(
                    [0, 1, 2].map(|i| ((u16::from(p[i]) * a + 255 * (255 - a) + 127) / 255) as u8),
                )
            });
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, 90)
                .encode_image(&rgb)
                .map_err(|_| IngestError::InvalidMedia)?;
            "image/jpeg"
        }
    };
    let bytes = buffer.into_inner();
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let token = format!(
        "{:x}",
        Sha256::digest(json!([key, sha256]).to_string().as_bytes())
    );
    let value = Cached {
        image: ServiceImage {
            bytes: Arc::from(bytes),
            mime_type,
        },
        native,
        display: dimensions,
        sha256,
        token,
        facts,
    };
    settings
        .preview_cache
        .lock()
        .map_err(|_| IngestError::Storage)?
        .insert(key, value.clone());
    Ok(Prepared { value, hit: false })
}
