use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ListArgs {
    pub collection: Option<String>,
    pub after_id: i64,
    pub limit: Option<usize>,
}

#[derive(Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewArgs {
    pub media_id: i64,
    #[serde(default)]
    pub original: bool,
    /// Optional detail tile: left, top, right, bottom in 0..1000.
    pub bbox_1000_xyxy: Option<[u32; 4]>,
}

#[derive(Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CropArgs {
    pub media_id: i64,
    pub expected_sha256: String,
    /// Coordinates are relative to the CURRENT image, not its display thumbnail.
    pub bbox_1000_xyxy: [u32; 4],
    #[serde(default)]
    pub apply: bool,
    pub reason: String,
    pub model: String,
}

#[derive(Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RestoreArgs {
    pub media_id: i64,
    pub expected_sha256: String,
    pub reason: String,
    pub model: String,
}

#[derive(Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnnotationArgs {
    pub media_id: i64,
    pub expected_sha256: String,
    /// Visual description only: identities are merged locally, artist first.
    pub caption: Option<String>,
    /// Generic visual tags only; spaces instead of underscores.
    pub tags: Option<Vec<String>>,
    pub text_regions: Option<Vec<Region>>,
    pub special_elements: Option<Vec<String>>,
    #[serde(default)]
    pub overwrite: bool,
    pub reason: String,
    pub model: String,
}

#[derive(Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub text: String,
    pub bbox_1000_xyxy: [i64; 4],
    pub kind: String,
    pub legibility: String,
}

#[derive(Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Keep,
    Reject,
    Defer,
}

#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssessArgs {
    pub media_id: i64,
    pub expected_sha256: String,
    /// 0..10, or null when no reliable judgement is possible.
    pub aesthetic: Option<f64>,
    pub training_suitability: Option<f64>,
    pub composition: Option<f64>,
    pub detail: Option<f64>,
    pub watermark_present: Option<bool>,
    pub style: String,
    pub decision: Decision,
    pub reason: String,
    pub model: String,
}

#[derive(Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScanArgs {
    /// At most 64 explicit IDs. Scanning never deletes library images.
    pub media_ids: Vec<i64>,
    pub similarity: Option<f32>,
}

#[derive(Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PairArgs {
    pub first_id: i64,
    pub second_id: i64,
    /// Same order as first_id, second_id.
    pub expected_sha256: [String; 2],
    /// Null means keep both; otherwise reject the other image into the queue.
    pub survivor_id: Option<i64>,
    pub reason: String,
    pub model: String,
}
