//! Transport-independent operations used by the headless CLI and MCP adapter.
use super::{params::*, tools, Settings, POLICY};
use crate::catalog::SqliteCatalog;
use bdm_ports::IngestError;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub struct ServiceImage {
    pub bytes: Arc<[u8]>,
    pub mime_type: &'static str,
}

#[derive(Debug)]
pub struct ServiceReply {
    pub metadata: Value,
    pub image: Option<ServiceImage>,
    pub is_error: bool,
}
impl ServiceReply {
    pub fn success(metadata: Value) -> Self {
        Self {
            metadata,
            image: None,
            is_error: false,
        }
    }
    pub fn error(metadata: Value) -> Self {
        Self {
            metadata,
            image: None,
            is_error: true,
        }
    }
}

#[derive(Clone)]
pub struct AgentService {
    pub(super) catalog: PathBuf,
    pub(super) settings: Arc<Settings>,
}
impl AgentService {
    pub(super) fn runtime_probe(&self) -> Value {
        use std::io::Read;
        use std::process::Stdio;
        let mut child=match crate::visual::python_command().args(["-c","import importlib.util,json; print(json.dumps({name:importlib.util.find_spec(name) is not None for name in ['torch','timm','PIL','numpy']}))"]).stdout(Stdio::piped()).stderr(Stdio::null()).spawn(){Ok(c)=>c,Err(_)=>return json!({"python_ready":false,"reason":"unavailable"})};
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut text = String::new();
                    if let Some(stdout) = child.stdout.take() {
                        let _ = stdout.take(4096).read_to_string(&mut text);
                    }
                    return json!({"python_ready":status.success(),"modules":serde_json::from_str::<Value>(&text).unwrap_or(Value::Null),"model_loaded":false});
                }
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(25))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return json!({"python_ready":false,"reason":"timeout_or_exit"});
                }
            }
        }
    }
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
            return Err("Existing catalog and absolute project/library paths required".into());
        }
        let repo = SqliteCatalog::connect_existing(catalog, !writes)?;
        if writes {
            repo.connection.execute_batch(super::jobs::SCHEMA)?;
            repo.connection.execute_batch(super::downloads::SCHEMA)?;
            repo.connection.execute_batch("CREATE INDEX IF NOT EXISTS collection_tag_page ON collections(tag,id); CREATE INDEX IF NOT EXISTS media_collections_page ON collection_media(media_id,collection_id);")?;
        }
        Ok(Self {
            catalog: catalog.canonicalize()?,
            settings: Arc::new(Settings {
                project_root: project_root.canonicalize()?,
                library_root: library_root.canonicalize()?,
                writes,
                permit: None,
                seen: Default::default(),
                previews: Default::default(),
                preview_cache: Default::default(),
            }),
        })
    }
    pub(super) fn connection(&self, read_only: bool) -> Result<SqliteCatalog, IngestError> {
        SqliteCatalog::connect_existing(&self.catalog, read_only || !self.settings.writes)
            .map_err(|_| IngestError::Storage)
    }
    pub fn policy(&self) -> ServiceReply {
        ServiceReply::success(
            json!({"protocol":"bdm-agent-v3","instructions":POLICY,"writes_enabled":self.settings.writes,
        "capabilities":["rich_text_regions","visual_regions","review_reasons","dataset_jsonl","crop_first_assessment","bucket_planning","jobs","work_leases","sql_paging","preset_crop_proposal","retained_element_boxes","tag_coverage_guard","typed_work_tools","display_preview_cache","compound_work","idempotent_claims"],
        "compound_tools":{"next":"bdm_work_next","apply_view":"bdm_work_apply_view","finish":"bdm_work_finish","inspection":"Inspect actual returned pixels before subsequent decisions; no crop-preview bypass","write_semantics":"Ordered recoverable phases, not atomic rollback; read assessment/annotation results independently"},
        "typed_tools":{"jobs":"bdm_job","downloads":"bdm_download","claim":"bdm_work_claim","view":"bdm_work_view","preview":"bdm_work_preview","apply":"bdm_work_apply","assess":"bdm_work_assess","annotate":"bdm_work_annotate","lease":"bdm_work_control","legacy":"bdm_agent"},
        "display_preview":{"new_work_tools_default":"1024-edge JPEG quality 90","legacy_default":"1024-edge PNG","max_edge_range":[256,2048],"memory_cache_max_bytes":33554432,"memory_cache_max_entries":32,"native_training_pixels_unchanged":true,"repeat_full_view":"Send known_preview_token only if this client retains the inspected preview; first view of a lease and changed current SHA always return pixels"},
        "crop_ratios":bdm_domain::training::RATIOS.iter().map(|r|r.0).collect::<Vec<_>>(),
        "image_retention":{"original":1,"current":1,"thumbnail":1},"next":{"action":"resolve_scope_then_submit_or_inspect"}}),
        )
    }
    pub fn execute(&self, operation: &str, input: Value) -> ServiceReply {
        let mut reply = match self.try_execute(operation, input.clone()) {
            Ok(r) => r,
            Err(e) => {
                let mut reply = tools::error_result(e);
                if operation.starts_with("work_") {
                    reply.metadata["next"]["operation"] = json!(operation);
                    reply.metadata["next"]["work_id"] = input["work_id"].clone();
                }
                reply
            }
        };
        if let Some(operation) = reply.metadata["next"]["operation"].as_str() {
            reply.metadata["next"]["tool"] = json!(super::operation_tool(operation));
        }
        reply
    }
    fn try_execute(&self, operation: &str, input: Value) -> Result<ServiceReply, IngestError> {
        macro_rules! args {($t:ty)=>{serde_json::from_value::<$t>(input.clone()).map_err(|e|IngestError::StorageDetails(json!({"code":"invalid_arguments","message":e.to_string(),"next":{"action":"check_schema"}}).to_string()))?};}
        let read_only = (operation == "character_repair" && input["apply"] != true)
            || matches!(
                operation,
                "collections"
                    | "list"
                    | "view"
                    | "bucket_plan"
                    | "doctor"
                    | "status"
                    | "jobs_list"
                    | "job_status"
                    | "job_plan"
                    | "policy"
                    | "download_status"
                    | "describe"
            );
        let mut repo = self.connection(read_only)?;
        let settings = &self.settings;
        match operation {
            "describe" => {
                let name = input["operation"].as_str().unwrap_or("");
                let mut schema=match name.trim_start_matches("work_") {
                    "view"=>serde_json::to_value(rmcp::schemars::schema_for!(ViewArgs)),
                    "preview"|"apply"|"crop"=>serde_json::to_value(rmcp::schemars::schema_for!(CropArgs)),
                    "annotate"=>serde_json::to_value(rmcp::schemars::schema_for!(AnnotationArgs)),
                    "assess"=>serde_json::to_value(rmcp::schemars::schema_for!(AssessArgs)),
                    _=>return Ok(ServiceReply::success(json!({"operation":name,"input_schema":super::operation_schema(name),"hint":"Typed MCP tools expose their complete schemas directly. CLI: bdm schema OPERATION requires no database."}))),
                }.map_err(|_|IngestError::Storage)?;
                if name.starts_with("work_") {
                    for key in [
                        "media_id",
                        "expected_sha256",
                        "overwrite",
                        "require_final_assessment",
                        "apply",
                    ] {
                        if let Some(props) = schema["properties"].as_object_mut() {
                            props.remove(key);
                        }
                        if let Some(required) = schema["required"].as_array_mut() {
                            required.retain(|v| v != key);
                        }
                    }
                }
                if matches!(
                    name.trim_start_matches("work_"),
                    "preview" | "apply" | "crop"
                ) {
                    if schema["required"].is_null() {
                        schema["required"] = json!([]);
                    }
                    schema["required"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!("ratio"));
                    if name == "work_apply" {
                        schema["required"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!("preview_sha256"));
                    }
                    schema["properties"]["ratio"] = json!({"type":"string","enum":bdm_domain::training::RATIOS.iter().map(|r|r.0).collect::<Vec<_>>()});
                }
                Ok(ServiceReply::success(
                    json!({"operation":name,"input_schema":super::operation_schema(name),"payload_schema":schema,"work_envelope":{"required":["work_id","lease_token","generation"],"writes_also_require":["request_id","expected_sha256","payload"]},"server_owned":["media_id","overwrite","require_final_assessment","apply","stage"]}),
                ))
            }
            "policy" => Ok(self.policy()),
            "character_repair" => {
                let apply = input["apply"].as_bool().unwrap_or(false);
                if apply && !settings.writes {
                    return Err(IngestError::StorageDetails(json!({"code":"read_only","message":"Restart with --allow-writes for prefix repair"}).to_string()));
                }
                let ids: Vec<i64> = serde_json::from_value(input["media_ids"].clone())
                    .map_err(|_| IngestError::InvalidMedia)?;
                for id in &ids {
                    let m = repo.media_by_id(*id)?.ok_or(IngestError::InvalidMedia)?;
                    tools::media(
                        &repo,
                        settings,
                        *id,
                        if apply { Some(&m.sha256) } else { None },
                    )?;
                }
                Ok(ServiceReply::success(
                    repo.repair_character_outputs(&ids, apply)?,
                ))
            }
            "collections" => tools::collections(&mut repo, settings, args!(ListArgs)),
            "list" => tools::list(&mut repo, settings, args!(ListArgs)),
            "view" => tools::view(&mut repo, settings, args!(ViewArgs)),
            "crop" => tools::crop(&mut repo, settings, args!(CropArgs)),
            "restore" => tools::restore(&mut repo, settings, args!(RestoreArgs)),
            "annotate" => tools::annotate(&mut repo, settings, args!(AnnotationArgs)),
            "assess" => tools::assess(&mut repo, settings, args!(AssessArgs)),
            "scan" => tools::scan(&mut repo, settings, args!(ScanArgs)),
            "pair" => tools::pair(&mut repo, settings, args!(PairArgs)),
            "work_next" | "work_apply_view" | "work_finish" => {
                super::composite::dispatch(self, &mut repo, operation, input)
            }
            "download_submit" | "download_control" | "download_status" => {
                super::downloads::dispatch(self, &mut repo, operation, input)
            }
            "bucket_plan" => {
                let id = input["media_id"]
                    .as_i64()
                    .ok_or(IngestError::InvalidMedia)?;
                let m = repo.media_by_id(id)?.ok_or(IngestError::InvalidMedia)?;
                let choices =
                    bdm_domain::training::bucket_choices(m.dimensions().0, m.dimensions().1);
                Ok(ServiceReply::success(
                    json!({"media_id":id,"sha256":m.sha256,"choices":choices,"native_pixels_unchanged":true,"next":{"tool":"bdm_view_image","hint":"Planning is geometry only; inspect composition before selecting a crop."}}),
                ))
            }
            _ => super::jobs::dispatch(self, &mut repo, operation, input),
        }
    }
}
