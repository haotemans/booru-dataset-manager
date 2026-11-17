//! Typed public envelopes. The existing service remains the sole workflow owner.
use super::{
    params::{AnnotationPayload, AssessmentPayload},
    preview::PreviewOptions,
};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkClaimArgs {
    /// Actual client/worker name. Do not claim more images than you can inspect.
    pub owner: String,
    pub job_id: Option<i64>,
    /// 1..16; default 1. Each image must still be viewed independently.
    pub limit: Option<u32>,
    /// Lease lifetime 30..1800 seconds; default 600.
    pub ttl_seconds: Option<u32>,
}

#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ViewPayload {
    pub original: bool,
    /// Detail tile in CURRENT image 0..1000 coordinates. A tile/original does
    /// not replace viewing the full current image before a visual decision.
    pub bbox_1000_xyxy: Option<[u32; 4]>,
    pub preview: PreviewOptions,
}
impl Default for ViewPayload {
    fn default() -> Self {
        Self {
            original: false,
            bbox_1000_xyxy: None,
            preview: PreviewOptions::compact(),
        }
    }
}

#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkViewArgs {
    pub work_id: i64,
    pub lease_token: String,
    pub generation: i64,
    #[serde(default)]
    pub payload: ViewPayload,
}

#[derive(Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
pub enum CropRatio {
    #[serde(rename = "21:9")]
    Wide21_9,
    #[serde(rename = "2:1")]
    Wide2_1,
    #[serde(rename = "16:9")]
    Wide16_9,
    #[serde(rename = "16:10")]
    Wide16_10,
    #[serde(rename = "3:2")]
    Wide3_2,
    #[serde(rename = "7:5")]
    Wide7_5,
    #[serde(rename = "4:3")]
    Wide4_3,
    #[serde(rename = "5:4")]
    Wide5_4,
    #[serde(rename = "1:1")]
    Square,
    #[serde(rename = "4:5")]
    Tall4_5,
    #[serde(rename = "3:4")]
    Tall3_4,
    #[serde(rename = "5:7")]
    Tall5_7,
    #[serde(rename = "2:3")]
    Tall2_3,
    #[serde(rename = "10:16")]
    Tall10_16,
    #[serde(rename = "9:16")]
    Tall9_16,
    #[serde(rename = "1:2")]
    Tall1_2,
    #[serde(rename = "9:21")]
    Tall9_21,
}

macro_rules! crop_payload {
    ($name:ident { $($extra:tt)* }) => {
        #[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            /// [left,top,right,bottom], integers 0..1000 relative to CURRENT pixels.
            pub bbox_1000_xyxy: [u32; 4],
            pub ratio: CropRatio,
            /// Explain how this proposal preserves the actual visible composition.
            pub reason: String,
            pub model: String,
            $($extra)*
        }
    };
}
crop_payload!(CropPreviewPayload {
    #[serde(default="PreviewOptions::compact")]
    pub preview: PreviewOptions,
});
crop_payload!(CropApplyPayload {
    /// Echo the exact inspected crop preview's SHA, never a display file hash.
    pub preview_sha256: String,
});

#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkPreviewArgs {
    pub work_id: i64,
    pub lease_token: String,
    pub generation: i64,
    pub expected_sha256: String,
    pub payload: CropPreviewPayload,
}

#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkWriteArgs<P> {
    pub work_id: i64,
    pub lease_token: String,
    pub generation: i64,
    pub expected_sha256: String,
    /// Unique for this exact write; reuse only when retrying identical arguments.
    pub request_id: String,
    pub payload: P,
}
pub type WorkApplyArgs = WorkWriteArgs<CropApplyPayload>;
pub type WorkAssessArgs = WorkWriteArgs<AssessmentPayload>;
pub type WorkAnnotateArgs = WorkWriteArgs<AnnotationPayload>;

#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkControlAction {
    Heartbeat,
    Release,
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkControlArgs {
    pub action: WorkControlAction,
    pub work_id: i64,
    pub lease_token: String,
    pub generation: i64,
}

#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobMode {
    Curate,
    Annotate,
    Bucket,
    Export,
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Caption,
    Tags,
    Regions,
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextKind {
    Caption,
    Tags,
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ExportLayout {
    Flat,
    MultiResolution,
}
macro_rules! job_plan_args {
    ($name:ident { $($extra:tt)* }) => {
        #[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            pub mode: JobMode,
            /// Supply 1..10000 distinct IDs OR an exact collection tag, never both.
            #[serde(skip_serializing_if="Option::is_none")]
            pub media_ids: Option<Vec<i64>>,
            #[serde(skip_serializing_if="Option::is_none")]
            pub collection: Option<String>,
            #[serde(skip_serializing_if="Option::is_none")]
            pub outputs: Option<Vec<OutputKind>>,
            #[serde(default)]
            pub overwrite: bool,
            /// Export only: absolute directory, text_kind required; flat by default.
            #[serde(skip_serializing_if="Option::is_none")]
            pub destination: Option<String>,
            #[serde(skip_serializing_if="Option::is_none")]
            pub text_kind: Option<TextKind>,
            #[serde(skip_serializing_if="Option::is_none")]
            pub export_layout: Option<ExportLayout>,
            #[serde(default)]
            pub include_items: bool,
            $($extra)*
        }
    };
}
job_plan_args!(JobPlanArgs {});
job_plan_args!(JobSubmitArgs {
    pub request_id: String,
    #[serde(skip_serializing_if="Option::is_none")]
    pub expected_plan_sha256: Option<String>,
});
#[derive(Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct JobsListArgs {
    pub after_id: i64,
    pub limit: Option<u32>,
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobStatusArgs {
    pub job_id: i64,
    #[serde(default)]
    pub after_id: i64,
    pub limit: Option<u32>,
    #[serde(default)]
    pub include_plan: bool,
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobAction {
    Pause,
    Resume,
    Retry,
    Cancel,
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JobControlArgs {
    pub job_id: i64,
    pub action: JobAction,
}
#[derive(Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmptyArgs {}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(tag = "operation", content = "input", deny_unknown_fields)]
#[schemars(extend("type" = "object"))]
pub enum JobRequest {
    #[serde(rename = "job_plan")]
    Plan(JobPlanArgs),
    #[serde(rename = "job_submit")]
    Submit(JobSubmitArgs),
    #[serde(rename = "jobs_list")]
    List(JobsListArgs),
    #[serde(rename = "job_status")]
    Status(JobStatusArgs),
    #[serde(rename = "job_control")]
    Control(JobControlArgs),
    /// Executes local preflight only. Hosted inference remains client-owned.
    #[serde(rename = "jobs_tick")]
    Tick(EmptyArgs),
}

#[derive(Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownloadMode {
    #[default]
    Automatic,
    Danbooru,
    #[serde(alias = "safedanbooru")]
    Safebooru,
    Gelbooru,
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DownloadSubmitArgs {
    pub request_id: String,
    pub collection_id: i64,
    /// Safebooru is safebooru.donmai.us, General-only with no fallback.
    #[serde(default)]
    pub source: DownloadMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_short_side: Option<u32>,
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DownloadStatusArgs {
    pub download_id: i64,
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownloadAction {
    Pause,
    Resume,
    Retry,
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DownloadControlArgs {
    pub download_id: i64,
    pub action: DownloadAction,
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(tag = "operation", content = "input", deny_unknown_fields)]
#[schemars(extend("type" = "object"))]
pub enum DownloadRequest {
    #[serde(rename = "download_submit")]
    Submit(DownloadSubmitArgs),
    #[serde(rename = "download_status")]
    Status(DownloadStatusArgs),
    #[serde(rename = "download_control")]
    Control(DownloadControlArgs),
}

#[derive(Clone, Default, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DoctorArgs {
    pub probe_runtime: bool,
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaIdArgs {
    pub media_id: i64,
}
#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkLeaseArgs {
    pub work_id: i64,
    pub lease_token: String,
    pub generation: i64,
}

pub const SCHEMA_OPERATIONS: &[&str] = &[
    "policy",
    "doctor",
    "status",
    "collections",
    "list",
    "view",
    "crop",
    "restore",
    "annotate",
    "assess",
    "scan",
    "pair",
    "bucket_plan",
    "job_plan",
    "job_submit",
    "jobs_list",
    "job_status",
    "job_control",
    "jobs_tick",
    "download_submit",
    "download_status",
    "download_control",
    "work_claim",
    "work_view",
    "work_preview",
    "work_apply",
    "work_assess",
    "work_annotate",
    "work_heartbeat",
    "work_release",
];

/// Pure schema discovery: no catalog, filesystem, credentials or network access.
pub fn operation_schema(operation: &str) -> Option<serde_json::Value> {
    use super::params::*;
    macro_rules! schema {
        ($t:ty) => {
            serde_json::to_value(schemars::schema_for!($t)).ok()
        };
    }
    match operation {
        "policy" | "status" | "jobs_tick" => schema!(EmptyArgs),
        "doctor" => schema!(DoctorArgs),
        "collections" | "list" => schema!(ListArgs),
        "view" => schema!(ViewArgs),
        "crop" => schema!(CropArgs),
        "restore" => schema!(RestoreArgs),
        "annotate" => schema!(AnnotationArgs),
        "assess" => schema!(AssessArgs),
        "scan" => schema!(ScanArgs),
        "pair" => schema!(PairArgs),
        "bucket_plan" => schema!(MediaIdArgs),
        "job_plan" => schema!(JobPlanArgs),
        "job_submit" => schema!(JobSubmitArgs),
        "jobs_list" => schema!(JobsListArgs),
        "job_status" => schema!(JobStatusArgs),
        "job_control" => schema!(JobControlArgs),
        "download_submit" => schema!(DownloadSubmitArgs),
        "download_status" => schema!(DownloadStatusArgs),
        "download_control" => schema!(DownloadControlArgs),
        "work_claim" => schema!(WorkClaimArgs),
        "work_view" => schema!(WorkViewArgs),
        "work_preview" => schema!(WorkPreviewArgs),
        "work_apply" => schema!(WorkApplyArgs),
        "work_assess" => schema!(WorkAssessArgs),
        "work_annotate" => schema!(WorkAnnotateArgs),
        "work_heartbeat" | "work_release" => schema!(WorkLeaseArgs),
        _ => None,
    }
}
