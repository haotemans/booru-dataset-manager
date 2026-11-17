//! Local stdio MCP adapter. The client supplies the vision/reasoning model.
mod control_params;
pub(crate) mod downloads;
pub mod jobs;
mod params;
mod preview;
pub mod service;
mod state;
mod tools;
pub use service::{AgentService, ServiceReply};
pub use state::{snapshot, AgentSnapshot};
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transport_tests;

use crate::catalog::SqliteCatalog;
use bdm_ports::IngestError;
use control_params::*;
pub use control_params::{operation_schema, SCHEMA_OPERATIONS};
use params::*;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

fn mcp_reply(reply: ServiceReply) -> CallToolResult {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let mut contents = vec![ContentBlock::text(reply.metadata.to_string())];
    if let Some(image) = reply.image {
        contents.push(ContentBlock::image(
            STANDARD.encode(image.bytes.as_ref()),
            image.mime_type,
        ));
    }
    if reply.is_error {
        CallToolResult::error(contents)
    } else {
        CallToolResult::success(contents)
    }
}

pub const POLICY: &str = include_str!("policy.txt");

#[derive(Clone)]
pub struct AgentServer {
    service: AgentService,
    tool_router: ToolRouter<Self>,
}

type CropPreviews = std::collections::BTreeMap<(i64, String, [u32; 4]), (String, String)>;
struct Settings {
    project_root: PathBuf,
    library_root: PathBuf,
    writes: bool,
    permit: Option<(i64, String, i64)>,
    seen: Mutex<std::collections::BTreeSet<String>>,
    previews: Mutex<CropPreviews>,
    preview_cache: std::sync::Arc<Mutex<preview::Cache>>,
}

impl AgentServer {
    async fn execute_operation(
        &self,
        operation: String,
        input: serde_json::Value,
    ) -> CallToolResult {
        let service = self.service.clone();
        match tokio::task::spawn_blocking(move || service.execute(&operation, input)).await {
            Ok(reply) => mcp_reply(reply),
            Err(_) => mcp_reply(ServiceReply::error(
                json!({"error":{"code":"worker_failed","message":"Inspect job status before retrying a write"}}),
            )),
        }
    }

    pub fn open(
        catalog: &Path,
        project_root: &Path,
        library_root: &Path,
        writes: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            service: AgentService::open(catalog, project_root, library_root, writes)?,
            tool_router: Self::tool_router(),
        })
    }

    async fn task<F>(&self, read_only: bool, task: F) -> CallToolResult
    where
        F: FnOnce(&mut SqliteCatalog, &Settings) -> Result<ServiceReply, IngestError>
            + Send
            + 'static,
    {
        let service = self.service.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let mut repo = service.connection(read_only)?;
            let settings = &service.settings;
            let outcome = task(&mut repo, settings);
            if let Err(error) = &outcome {
                // Keep validation failures diagnosable without recording pixels,
                // credentials or the full submitted training text.
                if settings.writes {
                    let _ = repo.audit(
                        "agent_tool_error",
                        &json!({"error":error.to_string()}).to_string(),
                    );
                }
            }
            outcome
        })
        .await;
        match outcome {
            Ok(Ok(result)) => mcp_reply(result),
            Ok(Err(error)) => mcp_reply(tools::error_result(error)),
            Err(_) => CallToolResult::error(vec![ContentBlock::text(
                "Local tool failed; inspect the image again before retrying.",
            )]),
        }
    }
}

fn operation_tool(operation: &str) -> &'static str {
    match operation {
        "work_claim" => "bdm_work_claim",
        "work_view" => "bdm_work_view",
        "work_preview" => "bdm_work_preview",
        "work_apply" => "bdm_work_apply",
        "work_assess" => "bdm_work_assess",
        "work_annotate" => "bdm_work_annotate",
        "work_heartbeat" | "work_release" => "bdm_work_control",
        "job_plan" | "job_submit" | "jobs_list" | "job_status" | "job_control" | "jobs_tick" => {
            "bdm_job"
        }
        "download_submit" | "download_status" | "download_control" => "bdm_download",
        _ => "bdm_agent",
    }
}

#[tool_router]
impl AgentServer {
    #[tool(
        description = "Plan, submit, inspect or control durable jobs. Each operation has its exact typed input in this schema. Read policy; plan/submit use media_ids OR an exact collection. Resume requested outputs only by default. jobs_tick runs local preflight, never hosted inference."
    )]
    async fn bdm_job(&self, Parameters(args): Parameters<JobRequest>) -> CallToolResult {
        let request = json!(args);
        self.execute_operation(
            request["operation"].as_str().unwrap().into(),
            request["input"].clone(),
        )
        .await
    }

    #[tool(
        description = "Submit/control source downloads through existing collections; inspect progress by download_id. Safebooru is the General-only Danbooru entrance, with shared credentials/IDs and no unrestricted fallback. Network execution starts in the daemon/jobs_tick."
    )]
    async fn bdm_download(&self, Parameters(args): Parameters<DownloadRequest>) -> CallToolResult {
        let request = json!(args);
        self.execute_operation(
            request["operation"].as_str().unwrap().into(),
            request["input"].clone(),
        )
        .await
    }

    #[tool(
        description = "Claim a small batch of independent leased images after local preflight. Each item returns work_id, lease_token, generation, expected_sha256 and stage. View each current image before deciding; never substitute source tags for inspection."
    )]
    async fn bdm_work_claim(&self, Parameters(args): Parameters<WorkClaimArgs>) -> CallToolResult {
        self.execute_operation("work_claim".into(), json!(args))
            .await
    }

    #[tool(
        description = "See a leased current image; first full view always sends pixels. Default display is 1024-edge JPEG, stored training pixels unchanged. Use PNG or 2048/detail tiles for small text. An original/tile does not authorize unseen current pixels. known_preview_token suppresses a repeat only after this lease's full view; omit it if the client lost the image."
    )]
    async fn bdm_work_view(&self, Parameters(args): Parameters<WorkViewArgs>) -> CallToolResult {
        self.execute_operation("work_view".into(), json!(args))
            .await
    }

    #[tool(
        description = "Propose and SEE a crop after inspecting the full current image, in the server's crop stage. Select one exact preset ratio and CURRENT-image bbox; preserve expression/composition, crop edge watermarks only when safe. Returns preview_sha256; inspect the returned image before apply. Display encoding never changes native PNG crop pixels."
    )]
    async fn bdm_work_preview(
        &self,
        Parameters(args): Parameters<WorkPreviewArgs>,
    ) -> CallToolResult {
        self.execute_operation("work_preview".into(), json!(args))
            .await
    }

    #[tool(
        description = "Apply the exact inspected crop proposal, echoing its ratio/bbox/preview_sha256. Saves native-pixel PNG with the verified original preserved. Never guess a preview hash. After saving, view the NEW current image before final assessment or annotation."
    )]
    async fn bdm_work_apply(&self, Parameters(args): Parameters<WorkApplyArgs>) -> CallToolResult {
        self.execute_operation("work_apply".into(), json!(args))
            .await
    }

    #[tool(
        description = "Assess inspected leased current pixels. The SERVER owns preliminary/final stage. In triage set crop_candidate=true only for useful composition-preserving cropping; after a crop outcome report applied/not_improved with real evidence. Final KEEP requires retained_elements (including [] only after observing none). Reject queues human review, never deletes."
    )]
    async fn bdm_work_assess(
        &self,
        Parameters(args): Parameters<WorkAssessArgs>,
    ) -> CallToolResult {
        self.execute_operation("work_assess".into(), json!(args))
            .await
    }

    #[tool(
        description = "Save requested caption/tags/regions for the inspected final image. The SERVER owns image identity, overwrite policy and assessment requirements. Artist/character names merge locally; produce visual content only. Remove redundancy while preserving distinct visible detail; compare view.tag_vocabulary.baseline_tags. Omitted regions preserve, [] clears. Same-call verification/readback; correct only rejected/missing outputs with a new request_id."
    )]
    async fn bdm_work_annotate(
        &self,
        Parameters(args): Parameters<WorkAnnotateArgs>,
    ) -> CallToolResult {
        self.execute_operation("work_annotate".into(), json!(args))
            .await
    }

    #[tool(
        description = "Extend this lease (heartbeat) or release it without a decision. Does not bypass the task stage or modify training images/TXT."
    )]
    async fn bdm_work_control(
        &self,
        Parameters(args): Parameters<WorkControlArgs>,
    ) -> CallToolResult {
        let operation = match args.action {
            WorkControlAction::Heartbeat => "work_heartbeat",
            WorkControlAction::Release => "work_release",
        };
        let mut input = json!(args);
        input.as_object_mut().unwrap().remove("action");
        self.execute_operation(operation.into(), input).await
    }

    #[tool(
        description = "Legacy operation/input adapter retained for compatibility. Prefer bdm_job, bdm_download and the typed bdm_work_* tools: their schemas expose exact arguments without describe. Legacy describe and doctor/status/bucket_plan/character_repair remain available. The same service enforces leases, stages, hashes and idempotency for every entry."
    )]
    async fn bdm_agent(&self, Parameters(args): Parameters<OperationArgs>) -> CallToolResult {
        if !matches!(
            args.operation.as_str(),
            "describe"
                | "character_repair"
                | "doctor"
                | "download_submit"
                | "download_status"
                | "download_control"
                | "bucket_plan"
                | "job_plan"
                | "job_submit"
                | "jobs_list"
                | "job_status"
                | "job_control"
                | "jobs_tick"
                | "work_claim"
                | "work_view"
                | "work_preview"
                | "work_apply"
                | "work_assess"
                | "work_annotate"
                | "work_heartbeat"
                | "work_release"
        ) {
            return mcp_reply(ServiceReply::error(
                json!({"error":{"code":"unknown_operation","message":"Use one of the documented control-plane operations"}}),
            ));
        }
        self.execute_operation(
            args.operation,
            if args.input.is_null() {
                json!({})
            } else {
                args.input
            },
        )
        .await
    }

    #[tool(
        description = "Read dataset workflow and mandatory preservation rules before acting.",
        annotations(read_only_hint = true)
    )]
    fn bdm_policy(&self) -> CallToolResult {
        mcp_reply(self.service.policy())
    }

    #[tool(
        description = "Page artist/character/query collections and image counts. Optional collection is a name substring; continue with next_after_id.",
        annotations(read_only_hint = true)
    )]
    async fn bdm_collections(&self, Parameters(args): Parameters<ListArgs>) -> CallToolResult {
        self.task(true, move |repo, settings| {
            tools::collections(repo, settings, args)
        })
        .await
    }

    #[tool(
        description = "Page through images by collection tag. Returns IDs, current hashes, dimensions, annotation and agent review states.",
        annotations(read_only_hint = true)
    )]
    async fn bdm_list_media(&self, Parameters(args): Parameters<ListArgs>) -> CallToolResult {
        self.task(true, move |repo, settings| {
            tools::list(repo, settings, args)
        })
        .await
    }

    #[tool(
        description = "Actually see the current image or complete original as an MCP image. Optional 0..1000 detail tile helps inspect small text. Display resizing never changes stored pixels.",
        annotations(read_only_hint = true)
    )]
    async fn bdm_view_image(&self, Parameters(args): Parameters<ViewArgs>) -> CallToolResult {
        self.task(true, move |repo, settings| {
            tools::view(repo, settings, args)
        })
        .await
    }

    #[tool(
        description = "Propose a crop AFTER visually inspecting the current image. Choose ratio from policy.crop_ratios and bbox in current-image 0..1000 coordinates, with a composition-specific reason. apply=false returns exact PNG preview and preview_sha256; inspect it before apply=true. Native PNG pixels are retained with 32 alignment, original preserved. Prefer removing a safely removable edge watermark; retain/BBox embedded text or watermarks when removal harms composition."
    )]
    async fn bdm_crop(&self, Parameters(args): Parameters<CropArgs>) -> CallToolResult {
        self.task(!args.apply, move |repo, settings| {
            tools::crop(repo, settings, args)
        })
        .await
    }

    #[tool(
        description = "Restore the complete verified original when expanding a previous crop. This intentionally replaces the current edit; inspect the original and save your desired crop coordinates first."
    )]
    async fn bdm_restore_original(
        &self,
        Parameters(args): Parameters<RestoreArgs>,
    ) -> CallToolResult {
        self.task(false, move |repo, settings| {
            tools::restore(repo, settings, args)
        })
        .await
    }

    #[tool(
        description = "Save caption/tags and optional rich text/visual regions for an inspected CURRENT image, or submit review_reasons only to defer it. Use view.tag_vocabulary; identities merge locally. Invalid terms have per-tag reasons and do not discard valid caption. Same-call readback, verification and next. Omitted region fields preserve; [] clears. overwrite=false preserves successful/manual TXT."
    )]
    async fn bdm_save_annotation(
        &self,
        Parameters(args): Parameters<AnnotationArgs>,
    ) -> CallToolResult {
        self.task(false, move |repo, settings| {
            tools::annotate(repo, settings, args)
        })
        .await
    }

    #[tool(
        description = "Assess inspected current pixels. Preliminary crop_candidate=true defers routing until a useful crop is previewed and the final image reassessed. Do not force previews when no composition-preserving watermark removal is feasible. Final KEEP in durable curation requires retained_elements (text/watermark/sticker/overlay/logo/signature, [] only after inspection found none); matching final-image BBoxes are required when annotating. crop_outcome=applied/not_improved needs real edit/preview evidence. Reject only queues human review."
    )]
    async fn bdm_assess(&self, Parameters(args): Parameters<AssessArgs>) -> CallToolResult {
        self.task(false, move |repo, settings| {
            tools::assess(repo, settings, args)
        })
        .await
    }

    #[tool(
        description = "Run local pHash/DINOv3 candidate analysis on up to 64 explicit image IDs. Never automatically deletes anything. Visually inspect candidate pairs before resolving; similar characters alone are not duplicates."
    )]
    async fn bdm_scan_duplicates(&self, Parameters(args): Parameters<ScanArgs>) -> CallToolResult {
        self.task(false, move |repo, settings| {
            tools::scan(repo, settings, args)
        })
        .await
    }

    #[tool(
        description = "Resolve a visually inspected duplicate pair using both fresh image hashes. Select one survivor to queue the other, or null to keep both. Collection associations are preserved when queued duplicates are finally deleted."
    )]
    async fn bdm_resolve_duplicates(
        &self,
        Parameters(args): Parameters<PairArgs>,
    ) -> CallToolResult {
        self.task(false, move |repo, settings| {
            tools::pair(repo, settings, args)
        })
        .await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AgentServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(POLICY)
    }
}

pub fn serve_stdio(
    catalog: &Path,
    project_root: &Path,
    library_root: &Path,
    writes: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let server = AgentServer::open(catalog, project_root, library_root, writes)?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            server
                .serve(rmcp::transport::stdio())
                .await?
                .waiting()
                .await?;
            Ok::<(), Box<dyn std::error::Error>>(())
        })
}
