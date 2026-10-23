use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};

pub const MAXIMUM_SHORT_SIDE: u32 = 16384;

pub fn default_minimum_short_side() -> u32 {
    crate::media::Dimensions::MINIMUM_SIDE
}

pub fn valid_minimum_short_side(value: u32) -> bool {
    (1..=MAXIMUM_SHORT_SIDE).contains(&value)
}

pub fn default_flatten_transparency() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourcePost {
    pub id: i64,
    #[serde(default, deserialize_with = "nullable_string")]
    pub md5: String,
    pub file_url: Option<String>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub file_ext: String,
    pub file_size: u64,
    pub image_width: u32,
    pub image_height: u32,
    pub tag_string: String,
    pub tag_string_artist: String,
    pub tag_string_character: String,
    #[serde(default)]
    pub tag_string_general: String,
    #[serde(default)]
    pub tag_string_copyright: String,
    #[serde(default)]
    pub tag_string_meta: String,
    pub rating: String,
    pub is_deleted: bool,
}
fn nullable_string<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

impl SourcePost {
    pub fn skip_reason(&self) -> Option<&'static str> {
        self.skip_reason_with_minimum(default_minimum_short_side())
    }

    pub fn skip_reason_with_minimum(&self, minimum_short_side: u32) -> Option<&'static str> {
        if self.is_deleted || self.file_url.is_none() {
            Some("source_unavailable")
        } else if !matches!(self.file_ext.as_str(), "jpg" | "jpeg" | "png" | "webp") {
            Some("unsupported_media_type")
        } else if self.tag_string.split_whitespace().any(|tag| {
            matches!(
                tag,
                "animated" | "animated_png" | "animated_webp" | "animation"
            )
        }) {
            Some("animated_media")
        } else if !valid_minimum_short_side(minimum_short_side)
            || self.image_width < minimum_short_side
            || self.image_height < minimum_short_side
        {
            Some("resolution_below_minimum")
        } else if self.file_size == 0 || self.file_size > 64 * 1024 * 1024 {
            Some("file_size_limit")
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredMedia {
    pub id: i64,
    pub post: SourcePost,
    pub path: PathBuf,
    pub thumbnail: PathBuf,
    pub sha256: String,
    /// SHA-256 of the downloaded source bytes, before optional normalization.
    #[serde(default)]
    pub source_sha256: String,
    pub collections: BTreeSet<i64>,
    pub keep: bool,
    #[serde(default = "default_minimum_short_side")]
    pub minimum_short_side: u32,
    #[serde(default = "default_flatten_transparency")]
    pub flatten_transparency: bool,
    #[serde(default)]
    pub edit: Option<crate::editing::EditRecord>,
}

impl StoredMedia {
    pub fn dimensions(&self) -> (u32, u32) {
        self.edit
            .as_ref()
            .and_then(|e| e.dimensions)
            .map_or((self.post.image_width, self.post.image_height), |d| {
                (d[0], d[1])
            })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IngestProgress {
    pub examined: u64,
    pub imported: u64,
    pub duplicates: u64,
    pub skipped: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestJob {
    pub id: i64,
    pub collection_id: i64,
    pub tag: String,
    pub root: PathBuf,
    #[serde(default = "default_minimum_short_side")]
    pub minimum_short_side: u32,
    #[serde(default = "default_flatten_transparency")]
    pub flatten_transparency: bool,
    pub before_id: Option<i64>,
    pub state: String,
    pub progress: IngestProgress,
    pub error: Option<String>,
    #[serde(default)]
    pub failed_posts: Vec<SourcePost>,
}
