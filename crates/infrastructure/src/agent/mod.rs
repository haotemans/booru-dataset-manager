//! Local stdio MCP adapter. The client supplies the vision/reasoning model.
mod params;
mod state;
mod tools;
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
    sync::{Arc, Mutex},
};

pub const POLICY: &str = include_str!("policy.txt");

#[derive(Clone)]
pub struct AgentServer {
    repo: Arc<Mutex<SqliteCatalog>>,
    settings: Arc<Settings>,
    tool_router: ToolRouter<Self>,
}

struct Settings {
    project_root: PathBuf,
    library_root: PathBuf,
    writes: bool,
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
        if !catalog.is_absolute()
            || !project_root.is_absolute()
            || !library_root.is_absolute()
            || !catalog.is_file()
        {
            return Err(
                "MCP requires an existing catalog and absolute project/library paths".into(),
            );
        }
        Ok(Self {
            repo: Arc::new(Mutex::new(SqliteCatalog::open(catalog)?)),
            settings: Arc::new(Settings {
                project_root: project_root.canonicalize()?,
                library_root: library_root.canonicalize()?,
                writes,
                seen: Mutex::default(),
                previews: Mutex::default(),
            }),
            tool_router: Self::tool_router(),
        })
    }

    async fn task<F>(&self, task: F) -> CallToolResult
    where
        F: FnOnce(&mut SqliteCatalog, &Settings) -> Result<CallToolResult, IngestError>
            + Send
            + 'static,
    {
        let repo = self.repo.clone();
        let settings = self.settings.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let mut repo = repo.lock().map_err(|_| IngestError::Storage)?;
            task(&mut repo, &settings)
        })
        .await;
        match outcome {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
            Err(_) => CallToolResult::error(vec![ContentBlock::text(
                "Local tool failed; inspect the image again before retrying.",
            )]),
        }
    }
}

#[tool_router]
impl AgentServer {
    #[tool(
        description = "Read dataset workflow and mandatory preservation rules before acting.",
        annotations(read_only_hint = true)
    )]
    fn bdm_policy(&self) -> CallToolResult {
        tools::result(
            json!({"instructions":POLICY,"writes_enabled":self.settings.writes,"image_retention":{"original":1,"current":1,"thumbnail":1}}),
        )
    }

    #[tool(
        description = "List artist/character/query collections and their image counts.",
        annotations(read_only_hint = true)
    )]
    async fn bdm_collections(&self) -> CallToolResult {
        self.task(tools::collections).await
    }

    #[tool(
        description = "Page through images by collection tag. Returns IDs, current hashes, dimensions, annotation and agent review states.",
        annotations(read_only_hint = true)
    )]
    async fn bdm_list_media(&self, Parameters(args): Parameters<ListArgs>) -> CallToolResult {
        self.task(move |repo, settings| tools::list(repo, settings, args))
            .await
    }

    #[tool(
        description = "Actually see the current image or complete original as an MCP image. Optional 0..1000 detail tile helps inspect small text. Display resizing never changes stored pixels.",
        annotations(read_only_hint = true)
    )]
    async fn bdm_view_image(&self, Parameters(args): Parameters<ViewArgs>) -> CallToolResult {
        self.task(move |repo, settings| tools::view(repo, settings, args))
            .await
    }

    #[tool(
        description = "Preview or APPLY a crop of the CURRENT image using normalized left/top/right/bottom coordinates. apply=false previews only. apply=true saves native-pixel PNG with 32 alignment and preserves the complete original. Inspect preview and use the fresh hash."
    )]
    async fn bdm_crop(&self, Parameters(args): Parameters<CropArgs>) -> CallToolResult {
        self.task(move |repo, settings| tools::crop(repo, settings, args))
            .await
    }

    #[tool(
        description = "Restore the complete verified original when expanding a previous crop. This intentionally replaces the current edit; inspect the original and save your desired crop coordinates first."
    )]
    async fn bdm_restore_original(
        &self,
        Parameters(args): Parameters<RestoreArgs>,
    ) -> CallToolResult {
        self.task(move |repo, settings| tools::restore(repo, settings, args))
            .await
    }

    #[tool(
        description = "Save visual caption/tags for the CURRENT image. Artist and character prefixes are merged locally; successful/manual text is preserved unless overwrite=true. Never changes raw API/TXT source data or calls another paid API."
    )]
    async fn bdm_save_annotation(
        &self,
        Parameters(args): Parameters<AnnotationArgs>,
    ) -> CallToolResult {
        self.task(move |repo, settings| tools::annotate(repo, settings, args))
            .await
    }

    #[tool(
        description = "Record explainable aesthetic/training scores and keep/reject/defer decision for an inspected image. Reject queues the image for final deletion review; it does not delete the original."
    )]
    async fn bdm_assess(&self, Parameters(args): Parameters<AssessArgs>) -> CallToolResult {
        self.task(move |repo, settings| tools::assess(repo, settings, args))
            .await
    }

    #[tool(
        description = "Run local pHash/DINOv3 candidate analysis on up to 64 explicit image IDs. Never automatically deletes anything. Visually inspect candidate pairs before resolving; similar characters alone are not duplicates."
    )]
    async fn bdm_scan_duplicates(&self, Parameters(args): Parameters<ScanArgs>) -> CallToolResult {
        self.task(move |repo, settings| tools::scan(repo, settings, args))
            .await
    }

    #[tool(
        description = "Resolve a visually inspected duplicate pair using both fresh image hashes. Select one survivor to queue the other, or null to keep both. Collection associations are preserved when queued duplicates are finally deleted."
    )]
    async fn bdm_resolve_duplicates(
        &self,
        Parameters(args): Parameters<PairArgs>,
    ) -> CallToolResult {
        self.task(move |repo, settings| tools::pair(repo, settings, args))
            .await
    }
}

#[tool_handler]
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
