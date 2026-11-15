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
    /// Visual description only: identities are merged locally, artist first. May be omitted with tags for a review_reasons-only decision.
    pub caption: Option<String>,
    /// Array of generic visual tags. Choose supported terms from view.tag_vocabulary; spaces instead of underscores. Unknown categories return per-tag reasons.
    pub tags: Option<Vec<String>>,
    /// Omit to preserve existing regions; [] explicitly clears. Coordinates refer to full current image.
    pub text_regions: Option<Vec<Region>>,
    /// Omit to preserve; [] clears. Only x_ray and cross_section are valid.
    pub special_elements: Option<Vec<String>>,
    /// Optional compound-composition regions. Omit to preserve; [] clears. Use regions for important insets/overlays, not mandatory segmentation of simple images.
    pub visual_regions: Option<Vec<crate::ai::VisualRegion>>,
    /// Reasons requiring human review, such as uncertain age in sexual content. Nonempty reasons defer the image and remove training approval; never delete it. Omit preserves.
    pub review_reasons: Option<Vec<String>>,
    /// Full curation only: require a current final KEEP assessment before writing caption/tags. Standalone annotation defaults false.
    #[serde(default)]
    pub require_final_assessment: bool,
    #[serde(default)]
    pub overwrite: bool,
    pub reason: String,
    pub model: String,
}

pub type Region = crate::ai::TextRegion;

#[derive(Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Keep,
    Reject,
    Defer,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentStage {
    Preliminary,
    #[default]
    Final,
}

#[derive(Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CropOutcome {
    /// Requires an actual crop whose parent hash is the pending original hash.
    Applied,
    /// Keep original composition only after viewing at least one exact crop preview at the same hash.
    NotImproved,
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
    /// Preliminary scores are provisional and never approve or reject. Only final results can route keep/reject.
    #[serde(default)]
    pub stage: AssessmentStage,
    /// Preliminary stage only. Defers final routing until a safe crop is tried and the actual result reassessed.
    #[serde(default)]
    pub crop_candidate: bool,
    /// Final stage after a pending crop: applied or not_improved. The server verifies the edit/preview evidence.
    pub crop_outcome: Option<CropOutcome>,
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
