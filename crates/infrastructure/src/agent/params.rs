use rmcp::schemars;
use serde::{Deserialize, Serialize};

/// Bounded control-plane operations. Business payloads use the same schemas as
/// the standalone tools; lease credentials never belong inside payload.
#[derive(Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationArgs {
    pub operation: String,
    #[serde(default)]
    pub input: serde_json::Value,
}

#[derive(Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ListArgs {
    pub collection: Option<String>,
    pub after_id: i64,
    pub limit: Option<usize>,
}

#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewArgs {
    pub media_id: i64,
    #[serde(default)]
    pub original: bool,
    /// Optional detail tile: left, top, right, bottom in 0..1000.
    pub bbox_1000_xyxy: Option<[u32; 4]>,
    /// Display-only options. Never resize or recompress stored training images.
    #[serde(default)]
    pub preview: super::preview::PreviewOptions,
}

#[derive(Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CropArgs {
    pub media_id: i64,
    pub expected_sha256: String,
    /// Coordinates are relative to the CURRENT image, not its display thumbnail.
    pub bbox_1000_xyxy: [u32; 4],
    /// Choose one of the existing 17 fixed ratios after inspecting composition.
    #[serde(default)]
    pub ratio: Option<String>,
    /// Echo the inspected preview's SHA when applying a durable crop proposal.
    #[serde(default)]
    pub preview_sha256: Option<String>,
    #[serde(default)]
    pub apply: bool,
    pub reason: String,
    pub model: String,
    /// Display-only crop preview options; saved crops remain native-pixel PNG.
    #[serde(default)]
    pub preview: super::preview::PreviewOptions,
}

#[derive(Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RestoreArgs {
    pub media_id: i64,
    pub expected_sha256: String,
    pub reason: String,
    pub model: String,
}

// Standalone and leased annotations share one field definition. Lease-only tools
// cannot accept identity/hash/overwrite fields inside their business payload.
macro_rules! annotation_args {
    ($name:ident { $($extra:tt)* }) => {
    #[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct $name {
    $($extra)*
    /// Visual description only; identities merge locally, artist first. Omit to preserve the existing caption while correcting other requested outputs.
    pub caption: Option<String>,
    /// Complete generic visual tag list. Redundancy cleanup is allowed; preserve distinct visible details. Compare view.tag_vocabulary.baseline_tags; large reductions are blocked for review. Spaces instead of underscores.
    pub tags: Option<Vec<String>>,
    /// Omit to preserve existing regions; [] explicitly clears. Coordinates refer to full current image.
    pub text_regions: Option<Vec<Region>>,
    /// Omit to preserve; [] clears. Only x_ray and cross_section are valid.
    pub special_elements: Option<Vec<String>>,
    /// Optional compound-composition regions. Omit to preserve; [] clears. Use regions for important insets/overlays, not mandatory segmentation of simple images.
    pub visual_regions: Option<Vec<crate::ai::VisualRegion>>,
    pub reason: String,
    pub model: String,
    }
    };
}
annotation_args!(AnnotationArgs {
    pub media_id: i64,
    pub expected_sha256: String,
    /// Nonempty reasons defer for human review; never delete the image.
    pub review_reasons: Option<Vec<String>>,
    #[serde(default)]
    pub require_final_assessment: bool,
    #[serde(default)]
    pub overwrite: bool,
});
annotation_args!(AnnotationPayload {});

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

#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RetainedElement {
    Text,
    Watermark,
    Sticker,
    Overlay,
    Logo,
    Signature,
}

macro_rules! assessment_args {
    ($name:ident { $($extra:tt)* }) => {
    #[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
    #[serde(deny_unknown_fields)]
    pub struct $name {
    $($extra)*
    /// 0..10, or null when no reliable judgement is possible.
    pub aesthetic: Option<f64>,
    pub training_suitability: Option<f64>,
    pub composition: Option<f64>,
    pub detail: Option<f64>,
    pub watermark_present: Option<bool>,
    /// Final current-image inspection inventory. [] means no visible text or overlays;
    /// every declared retained kind needs its own matching BBox evidence at annotation.
    #[serde(default)]
    pub retained_elements: Option<Vec<RetainedElement>>,
    pub style: String,
    pub decision: Decision,
    /// Preliminary stage only. Defers final routing until a safe crop is tried and the actual result reassessed.
    #[serde(default)]
    pub crop_candidate: bool,
    /// Final stage after a pending crop: applied or not_improved. The server verifies the edit/preview evidence.
    pub crop_outcome: Option<CropOutcome>,
    pub reason: String,
    pub model: String,
    }
    };
}
assessment_args!(AssessArgs {
    pub media_id: i64,
    pub expected_sha256: String,
    /// Preliminary results cannot approve or reject; durable jobs own this field.
    #[serde(default)]
    pub stage: AssessmentStage,
});
assessment_args!(AssessmentPayload {});

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
