//! Local stdio MCP adapter. The client supplies the vision/reasoning model.
pub(crate) mod downloads;
pub mod jobs;
mod params;
pub mod service;
mod state;
mod tools;
pub use service::{AgentService, ServiceReply};
pub use state::{snapshot, AgentSnapshot};
#[cfg(test)]
mod tests;

use crate::catalog::SqliteCatalog;
use bdm_ports::IngestError;
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
    if let Some(png) = reply.image_png {
        contents.push(ContentBlock::image(STANDARD.encode(png), "image/png"));
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

struct Settings {
    project_root: PathBuf,
    library_root: PathBuf,
    writes: bool,
    permit: Option<(i64, String, i64)>,
    seen: Mutex<std::collections::BTreeSet<String>>,
    previews: Mutex<std::collections::BTreeSet<(i64, String, [u32; 4])>>,
}

impl AgentServer {
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

#[tool_router]
impl AgentServer {
    #[tool(
        description = "Durable jobs and leased work. Read policy first; describe input {operation:work_annotate|work_assess|work_preview|work_view} returns exact payload schema once. operation: describe, doctor, download_submit, download_status, download_control, bucket_plan, job_plan, job_submit, jobs_list, job_status, job_control, jobs_tick, work_claim, work_view, work_preview, work_apply, work_assess, work_annotate, work_heartbeat, work_release. job_plan/submit input: mode curate|annotate|bucket|export, media_ids OR collection, outputs [caption,tags,regions], overwrite=false; submit also request_id. work_claim: owner, optional job_id/limit<=16. Other work_*: work_id, lease_token, generation, expected_sha256, payload (standalone tool schema without media_id/hash); writes also request_id. Follow next_stage; server enforces ordering. job_control: job_id, action pause|resume|retry|cancel. Export additionally destination absolute, text_kind caption|tags. download_submit input: request_id, existing collection_id, source automatic|danbooru|gelbooru, minimum_short_side (default800). download_status/control use download_id; control action pause|resume|retry. Local preflight via daemon or jobs_tick; model calls remain client-owned."
    )]
    async fn bdm_agent(&self, Parameters(args): Parameters<OperationArgs>) -> CallToolResult {
        if !matches!(
            args.operation.as_str(),
            "describe"
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
        let service = self.service.clone();
        match tokio::task::spawn_blocking(move || {
            service.execute(
                &args.operation,
                if args.input.is_null() {
                    json!({})
                } else {
                    args.input
                },
            )
        })
        .await
        {
            Ok(reply) => mcp_reply(reply),
            Err(_) => mcp_reply(ServiceReply::error(
                json!({"error":{"code":"worker_failed","message":"Inspect job status before retrying a write"}}),
            )),
        }
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
        description = "Preview or APPLY a crop of the CURRENT image using normalized left/top/right/bottom coordinates. apply=false previews only. apply=true saves native-pixel PNG with 32 alignment and preserves the complete original. Inspect preview and use the fresh hash."
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
        description = "Assess inspected current pixels. stage=preliminary never approves/rejects; crop_candidate=true defers routing until verified crop/preview and final reassessment. Final crop_outcome=applied or not_improved requires matching edit/preview evidence. Reject queues human review and removes training approval, never deletes bytes. Legacy calls default to final."
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
