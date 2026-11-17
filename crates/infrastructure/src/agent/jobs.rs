//! Durable local control plane. SQLite transactions fence Agent leases; the
//! daemon runs deterministic stages, never a hosted model or an external Agent.
use super::{
    params::*,
    service::{AgentService, ServiceReply},
    tools, Settings,
};
use crate::{catalog::SqliteCatalog, ingest::storage, workflows::WorkflowJob};
use bdm_ports::IngestError;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::atomic::AtomicBool,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const SCHEMA:&str="
CREATE TABLE IF NOT EXISTS agent_jobs(id INTEGER PRIMARY KEY REFERENCES workflow_jobs(id),request_id TEXT UNIQUE NOT NULL,fingerprint TEXT NOT NULL,plan TEXT NOT NULL,options TEXT NOT NULL,state TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS agent_work(id INTEGER PRIMARY KEY,job_id INTEGER NOT NULL REFERENCES agent_jobs(id),media_id INTEGER NOT NULL REFERENCES media(id),sha TEXT NOT NULL,revision TEXT NOT NULL,stage TEXT NOT NULL,lease TEXT,owner TEXT,expires INTEGER NOT NULL DEFAULT 0,generation INTEGER NOT NULL DEFAULT 0,busy INTEGER NOT NULL DEFAULT 0,viewed TEXT,preview TEXT,last_error TEXT,result TEXT);
CREATE UNIQUE INDEX IF NOT EXISTS agent_work_live_media ON agent_work(media_id) WHERE lease IS NOT NULL;
CREATE INDEX IF NOT EXISTS agent_work_ready ON agent_work(job_id,stage,id);
CREATE TABLE IF NOT EXISTS agent_receipts(work_id INTEGER NOT NULL,request_id TEXT NOT NULL,fingerprint TEXT NOT NULL,response TEXT,state TEXT NOT NULL,lease_proof TEXT NOT NULL,PRIMARY KEY(work_id,request_id));
CREATE TABLE IF NOT EXISTS agent_claims(request_id TEXT PRIMARY KEY,fingerprint TEXT NOT NULL,response TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS agent_daemon(id INTEGER PRIMARY KEY CHECK(id=1),pid INTEGER,heartbeat INTEGER NOT NULL DEFAULT 0,stop INTEGER NOT NULL DEFAULT 0);
";
pub(super) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn error(code: &str, message: &str) -> IngestError {
    IngestError::StorageDetails(
        json!({"code":code,"message":message,"next":{"action":"correct_or_resume_requested_work"}})
            .to_string(),
    )
}
fn number(v: &Value, key: &str) -> Result<i64, IngestError> {
    v[key]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or_else(|| error("invalid_arguments", key))
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, IngestError> {
    v[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
        .ok_or_else(|| error("invalid_arguments", key))
}
fn digest(v: &Value) -> String {
    format!("{:x}", Sha256::digest(v.to_string().as_bytes()))
}
pub(super) fn annotation_revision(
    repo: &SqliteCatalog,
    m: &bdm_domain::ingest::StoredMedia,
) -> Result<String, IngestError> {
    let mut q=storage(repo.connection.prepare("SELECT kind,content,state,reviewed,provenance FROM text_artifacts WHERE media_id=?1 AND is_current=1 ORDER BY kind"))?;
    let artifacts: Vec<(String, String, String, bool, String)> = storage(
        q.query_map([m.id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .and_then(|r| r.collect()),
    )?;
    let sidecar = |ext: &str| -> Result<Option<String>, IngestError> {
        match std::fs::read(m.path.with_extension(ext)) {
            Ok(bytes) => Ok(Some(format!("{:x}", Sha256::digest(bytes)))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(IngestError::Storage),
        }
    };
    let regions: Option<String> = storage(
        repo.connection
            .query_row(
                "SELECT provenance FROM media_quality WHERE media_id=?1",
                [m.id],
                |r| r.get(0),
            )
            .optional(),
    )?
    .flatten();
    let mut revision = json!([artifacts, sidecar("txt")?, sidecar("tags.txt")?, regions]);
    if !m.artist_scope.is_empty() {
        revision
            .as_array_mut()
            .unwrap()
            .push(json!({"artist_scope":m.artist_scope}));
    }
    Ok(digest(&revision))
}
fn output_complete(
    repo: &SqliteCatalog,
    m: &bdm_domain::ingest::StoredMedia,
    kind: &str,
) -> Result<bool, IngestError> {
    let row:Option<(String,String,bool,String)>=storage(repo.connection.query_row("SELECT content,state,reviewed,provenance FROM text_artifacts WHERE media_id=?1 AND kind=?2 AND is_current=1",params![m.id,kind],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional())?;
    let Some((content, state, reviewed, provenance)) = row else {
        return Ok(false);
    };
    let artifact = crate::workflows::TextArtifact {
        media_id: m.id,
        kind: kind.into(),
        content: content.clone(),
        state: state.clone(),
        reviewed,
        provenance,
    };
    Ok(state == "complete"
        && !content.trim().is_empty()
        && (kind == "caption" || artifact.is_generated_output() || reviewed)
        && std::fs::read(
            m.path
                .with_extension(if kind == "tags" { "tags.txt" } else { "txt" }),
        )
        .is_ok_and(|bytes| bytes == content.as_bytes()))
}
fn route(reply: &mut ServiceReply, id: i64, stage: &str, operation: &str) {
    reply.metadata["work_id"] = json!(id);
    reply.metadata["next_stage"] = json!(stage);
    reply.metadata["next"] = json!({"tool":super::operation_tool(operation),"operation":operation,
        "required":["work_id","lease_token","generation"],
        "hint":match operation {"work_view"=>"Inspect the full saved current image; reuse its new SHA.","work_preview"=>"Preview a composition-preserving crop; do not bypass the lease with standalone tools.","work_apply"=>"Apply this exact inspected preview, or assess with crop_outcome=not_improved if it harms expression.","work_assess"=>"Assess current visible pixels. Triage: crop_candidate=true if useful cropping is possible; after crop use verified crop_outcome.","work_annotate"=>"Submit only the requested outputs. Identity merging is local; omit already successful fields when correcting.",_=>"Continue claiming requested work; terminal results stay in history."}});
    if operation == "work_claim" {
        reply.metadata["next"]["required"] = json!(["owner"]);
    } else {
        reply.metadata["next"]["work_id"] = json!(id);
        if operation == "work_preview" {
            reply.metadata["next"]["required"] = json!([
                "work_id",
                "lease_token",
                "generation",
                "expected_sha256",
                "payload"
            ]);
        } else if matches!(operation, "work_apply" | "work_assess" | "work_annotate") {
            reply.metadata["next"]["write_required"] =
                json!(["request_id", "expected_sha256", "payload"]);
        }
    }
}
fn writable(service: &AgentService) -> Result<(), IngestError> {
    if service.settings.writes {
        Ok(())
    } else {
        Err(error(
            "read_only",
            "Restart with --allow-writes for mutations",
        ))
    }
}

pub fn plan(repo: &SqliteCatalog, input: &Value) -> Result<Value, IngestError> {
    if input.as_object().is_none_or(|o| {
        o.keys().any(|key| {
            ![
                "mode",
                "media_ids",
                "collection",
                "outputs",
                "overwrite",
                "destination",
                "text_kind",
                "request_id",
                "expected_plan_sha256",
                "export_layout",
                "include_items",
            ]
            .contains(&key.as_str())
        })
    }) {
        return Err(error(
            "invalid_arguments",
            "Unknown job field; consult job_plan schema and correct misspelled options",
        ));
    }
    let mode = input["mode"].as_str().unwrap_or("curate");
    if !matches!(mode, "curate" | "annotate" | "bucket" | "export") {
        return Err(error("unsupported_mode","Modes: curate, annotate, bucket, export. Download uses the existing ingestion command."));
    }
    let mut ids: BTreeSet<i64> = input["media_ids"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default();
    if input["media_ids"]
        .as_array()
        .is_some_and(|a| a.len() != ids.len() || ids.iter().any(|n| *n <= 0))
    {
        return Err(error("invalid_scope", "Use distinct positive IDs"));
    }
    if let Some(tag) = input["collection"].as_str() {
        if !ids.is_empty() {
            return Err(error("invalid_scope", "Use media_ids OR collection"));
        }
        let mut q=storage(repo.connection.prepare("SELECT DISTINCT cm.media_id FROM collections c JOIN collection_media cm ON cm.collection_id=c.id WHERE c.tag=?1 ORDER BY cm.media_id LIMIT 10001"))?;
        ids = storage(q.query_map([tag], |r| r.get(0)).and_then(|r| r.collect()))?;
    }
    if ids.is_empty() || ids.len() > 10000 {
        return Err(error("invalid_scope", "Select 1..10000 existing media IDs"));
    }
    let outputs = input["outputs"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![json!("caption"), json!("tags"), json!("regions")]);
    if outputs.is_empty()
        || outputs
            .iter()
            .any(|o| !matches!(o.as_str(), Some("caption" | "tags" | "regions")))
    {
        return Err(error("invalid_outputs", "Outputs: caption, tags, regions"));
    }
    let mut items = Vec::new();
    for id in &ids {
        let m = repo
            .media_by_id(*id)?
            .ok_or_else(|| error("media_not_found", "Scope contains missing media"))?;
        items.push(json!({"media_id":id,"sha256":m.sha256,"annotation_revision":annotation_revision(repo,&m)?}));
    }
    if mode == "export"
        && (!input["destination"]
            .as_str()
            .is_some_and(|s| Path::new(s).is_absolute())
            || !matches!(input["text_kind"].as_str(), Some("caption" | "tags")))
    {
        return Err(error(
            "invalid_export",
            "Absolute destination and text_kind caption/tags required",
        ));
    }
    let layout = input["export_layout"].as_str().unwrap_or("flat");
    if !matches!(layout, "flat" | "multi-resolution") {
        return Err(error(
            "invalid_export",
            "export_layout: flat or multi-resolution",
        ));
    }
    Ok(
        json!({"mode":mode,"items":items,"outputs":outputs,"overwrite":input["overwrite"].as_bool().unwrap_or(false),"destination":input["destination"],"text_kind":input["text_kind"],"export_layout":layout,"scope_count":ids.len(),"effects":if mode=="bucket"{"read_geometry_only"}else if mode=="export"{"managed_export"}else{"selected_annotations_and_review; crop only after explicit Agent preview/apply"}}),
    )
}

fn submit(repo: &mut SqliteCatalog, input: Value) -> Result<ServiceReply, IngestError> {
    let request = text(&input, "request_id")?;
    let mut intent = input.clone();
    intent.as_object_mut().unwrap().remove("request_id");
    let fingerprint = digest(&intent);
    let prior: Option<(i64, String)> = storage(
        repo.connection
            .query_row(
                "SELECT id,fingerprint FROM agent_jobs WHERE request_id=?1",
                [request],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional(),
    )?;
    if let Some((id, fp)) = prior {
        if fp != fingerprint {
            return Err(error(
                "idempotency_conflict",
                "request_id reused with different inputs",
            ));
        }
        return Ok(ServiceReply::success(json!({"job_id":id,"replayed":true})));
    }
    let plan = plan(repo, &input)?;
    if input["expected_plan_sha256"]
        .as_str()
        .is_some_and(|expected| expected != digest(&plan))
    {
        return Err(error(
            "plan_changed",
            "Scope or annotation changed after preview; inspect a new job_plan",
        ));
    }
    let mut options = plan.clone();
    options.as_object_mut().unwrap().remove("items");
    let tx = storage(
        repo.connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
    )?;
    if let Some((id, previous)) = storage(
        tx.query_row(
            "SELECT id,fingerprint FROM agent_jobs WHERE request_id=?1",
            [request],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .optional(),
    )? {
        if previous != fingerprint {
            return Err(error(
                "idempotency_conflict",
                "request_id reused with different plan",
            ));
        }
        return Ok(ServiceReply::success(json!({"job_id":id,"replayed":true})));
    }
    storage(tx.execute(
        "INSERT INTO workflow_jobs(kind,metadata) VALUES('agent-curation','{}')",
        [],
    ))?;
    let id = tx.last_insert_rowid();
    let job = WorkflowJob {
        id,
        kind: "agent-curation".into(),
        ids: plan["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|x| x["media_id"].as_i64())
            .collect(),
        state: "running".into(),
        mode: plan["mode"].as_str().unwrap().into(),
        ..Default::default()
    };
    storage(tx.execute(
        "UPDATE workflow_jobs SET metadata=?2 WHERE id=?1",
        params![id, serde_json::to_string(&job).unwrap()],
    ))?;
    storage(tx.execute(
        "INSERT INTO agent_jobs VALUES(?1,?2,?3,?4,?5,'running')",
        params![
            id,
            request,
            fingerprint,
            plan.to_string(),
            options.to_string()
        ],
    ))?;
    for item in plan["items"].as_array().unwrap() {
        storage(tx.execute(
            "INSERT INTO agent_work(job_id,media_id,sha,revision,stage) VALUES(?1,?2,?3,?4,'local')",
            params![id, item["media_id"].as_i64(), item["sha256"].as_str(),item["annotation_revision"].as_str()],
        ))?;
    }
    storage(tx.commit())?;
    repo.audit(
        "agent_job_event",
        &json!({"job_id":id,"state":"submitted"}).to_string(),
    )?;
    Ok(ServiceReply::success(
        json!({"job_id":id,"plan":options,"snapshot_sha256":digest(&plan),"state":"running","next":{"operation":"work_claim","hint":"Run daemon serve or jobs tick for local preflight; then claim Agent work."}}),
    ))
}

pub fn status(repo: &SqliteCatalog, id: i64) -> Result<Value, IngestError> {
    status_page(repo, id, 0, 64, false)
}
fn status_page(
    repo: &SqliteCatalog,
    id: i64,
    after: i64,
    limit: u64,
    include_plan: bool,
) -> Result<Value, IngestError> {
    let (state, raw): (String, String) = storage(repo.connection.query_row(
        "SELECT state,CASE WHEN ?2 THEN plan ELSE options END FROM agent_jobs WHERE id=?1",
        params![id, include_plan],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ))?;
    let mut plan: Value = serde_json::from_str(&raw).map_err(|_| IngestError::Storage)?;
    if !include_plan {
        plan.as_object_mut().unwrap().remove("items");
    }
    let mut q = storage(
        repo.connection
            .prepare("SELECT stage,count(*) FROM agent_work WHERE job_id=?1 GROUP BY stage"),
    )?;
    let counts: serde_json::Map<String, Value> = storage(
        q.query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, json!(r.get::<_, i64>(1)?)))
        })
        .and_then(|r| r.collect()),
    )?;
    let mut q=storage(repo.connection.prepare("SELECT id,media_id,sha,stage,owner,expires,generation,last_error,result FROM agent_work WHERE job_id=?1 AND id>?2 ORDER BY id LIMIT ?3"))?;
    let mut items:Vec<Value>=storage(q.query_map(params![id,after,limit+1],|r|Ok(json!({"work_id":r.get::<_,i64>(0)?,"media_id":r.get::<_,i64>(1)?,"sha256":r.get::<_,String>(2)?,"stage":r.get::<_,String>(3)?,"owner":r.get::<_,Option<String>>(4)?,"expires":r.get::<_,i64>(5)?,"generation":r.get::<_,i64>(6)?,"error":r.get::<_,Option<String>>(7)?,"result":r.get::<_,Option<String>>(8)?.and_then(|s|serde_json::from_str::<Value>(&s).ok())}))).and_then(|r|r.collect()))?;
    let more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next = if more {
        items.last().and_then(|i| i["work_id"].as_i64())
    } else {
        None
    };
    Ok(
        json!({"job_id":id,"state":state,"plan":plan,"counts":counts,"items":items,"next_after_id":next}),
    )
}
fn finish(repo: &mut SqliteCatalog, id: i64) -> Result<(), IngestError> {
    let tx = storage(
        repo.connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
    )?;
    let (current,total,finished,failed):(String,i64,i64,i64)=storage(tx.query_row("SELECT j.state,count(*),sum(w.stage IN ('completed','review','deferred','failed','recovery_required','cancelled')),sum(w.stage IN ('failed','recovery_required')) FROM agent_jobs j JOIN agent_work w ON w.job_id=j.id WHERE j.id=?1 GROUP BY j.id",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))))?;
    let terminal = total == finished;
    let stopped = matches!(current.as_str(), "paused" | "cancelled");
    let state = if terminal && !stopped {
        if failed > 0 {
            "failed"
        } else {
            "completed"
        }
    } else {
        &current
    };
    storage(tx.execute(
        "UPDATE agent_jobs SET state=?2 WHERE id=?1",
        params![id, state],
    ))?;
    let dismissed = (terminal && !stopped) || state == "cancelled";
    storage(tx.execute("UPDATE workflow_jobs SET metadata=json_set(metadata,'$.cursor',?2,'$.state',?3,'$.dismissed',json(?4)) WHERE id=?1",params![id,finished,state,if dismissed{"true"}else{"false"}]))?;
    if terminal {
        storage(tx.execute("UPDATE workflow_jobs SET metadata=json_set(metadata,'$.accepted',json((SELECT json_group_array(media_id) FROM agent_work WHERE job_id=?1 AND stage='completed')),'$.recovery',json((SELECT json_group_array(media_id) FROM agent_work WHERE job_id=?1 AND stage IN ('failed','recovery_required')))) WHERE id=?1",[id]))?;
    }
    storage(tx.execute(
        "INSERT INTO audit_log(action,metadata) VALUES('agent_job_event',?1)",
        [json!({"job_id":id,"terminal":terminal}).to_string()],
    ))?;
    storage(tx.commit())?;
    Ok(())
}

pub fn control_id(repo: &mut SqliteCatalog, action: &str, id: i64) -> Result<Value, IngestError> {
    control(repo, action, &json!({"job_id":id}))
}

pub fn managed_download(repo: &SqliteCatalog, id: i64) -> bool {
    super::downloads::managed(repo, id).unwrap_or(false)
}
pub fn control_download(
    repo: &mut SqliteCatalog,
    id: i64,
    action: &str,
) -> Result<Value, IngestError> {
    super::downloads::control(repo, id, action)
}

pub fn control(
    repo: &mut SqliteCatalog,
    action: &str,
    input: &Value,
) -> Result<Value, IngestError> {
    let id = number(input, "job_id")?;
    let tx = storage(
        repo.connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
    )?;
    let current: String = storage(tx.query_row(
        "SELECT state FROM agent_jobs WHERE id=?1",
        [id],
        |r| r.get(0),
    ))?;
    if (action == "resume" && current != "paused")
        || (action == "retry" && current != "failed")
        || (matches!(action, "pause" | "cancel")
            && !matches!(current.as_str(), "running" | "paused"))
    {
        return Err(error(
            "invalid_transition",
            "Job state does not permit this control action",
        ));
    }
    match action {
        "pause" | "cancel" => {
            storage(tx.execute(
                "UPDATE agent_jobs SET state=?2 WHERE id=?1",
                params![
                    id,
                    if action == "pause" {
                        "paused"
                    } else {
                        "cancelled"
                    }
                ],
            ))?;
            // In-flight short commits finish safely; their later result cannot revive cancellation.
            storage(tx.execute("UPDATE agent_work SET lease=NULL,owner=NULL,expires=0,generation=generation+1,stage=CASE WHEN ?2 THEN 'cancelled' ELSE stage END WHERE job_id=?1 AND busy=0",params![id,action=="cancel"]))?;
        }
        "resume" => {
            storage(tx.execute(
                "UPDATE agent_jobs SET state='running' WHERE id=?1 AND state='paused'",
                [id],
            ))?;
        }
        "retry" => {
            let ambiguous: i64 = storage(tx.query_row(
                "SELECT count(*) FROM agent_work WHERE job_id=?1 AND stage='recovery_required'",
                [id],
                |r| r.get(0),
            ))?;
            if ambiguous > 0 {
                return Err(error(
                    "recovery_required",
                    "Ambiguous writes require reconciliation, not blind replay",
                ));
            }
            storage(tx.execute("UPDATE agent_work SET stage='local',last_error=NULL,lease=NULL,owner=NULL,viewed=NULL,preview=NULL,generation=generation+1 WHERE job_id=?1 AND stage='failed' AND busy=0",[id]))?;
            storage(tx.execute("UPDATE agent_jobs SET state='running' WHERE id=?1", [id]))?;
        }
        _ => return Err(error("invalid_action", "pause, resume, cancel or retry")),
    }
    let raw: String = storage(tx.query_row(
        "SELECT metadata FROM workflow_jobs WHERE id=?1",
        [id],
        |r| r.get(0),
    ))?;
    let mut job: WorkflowJob = serde_json::from_str(&raw).map_err(|_| IngestError::Storage)?;
    job.state = if action == "pause" {
        "paused"
    } else if action == "cancel" {
        "cancelled"
    } else {
        "running"
    }
    .into();
    job.dismissed = action == "cancel";
    storage(tx.execute(
        "UPDATE workflow_jobs SET metadata=?2 WHERE id=?1",
        params![id, serde_json::to_string(&job).unwrap()],
    ))?;
    storage(tx.commit())?;
    repo.audit(
        "agent_job_event",
        &json!({"job_id":id,"action":action}).to_string(),
    )?;
    if action == "resume" {
        finish(repo, id)?;
    }
    Ok(json!({"job_id":id,"action":action}))
}

fn claim(repo: &mut SqliteCatalog, input: &Value) -> Result<ServiceReply, IngestError> {
    let owner = text(input, "owner")?;
    let limit = input["limit"].as_u64().unwrap_or(4).clamp(1, 16);
    let ttl = input["ttl_seconds"].as_i64().unwrap_or(600).clamp(30, 1800);
    let at = now();
    let tx = storage(
        repo.connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
    )?;
    let request = if input["request_id"].is_null() {
        None
    } else {
        Some(text(input, "request_id")?)
    };
    let fingerprint = digest(&json!([
        owner,
        input["job_id"].as_i64().unwrap_or(0),
        limit,
        ttl
    ]));
    if let Some(request) = request {
        let receipt: Option<(String, String)> = storage(
            tx.query_row(
                "SELECT fingerprint,response FROM agent_claims WHERE request_id=?1",
                [request],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional(),
        )?;
        if let Some((previous, response)) = receipt {
            if previous != fingerprint {
                return Err(error(
                    "idempotency_conflict",
                    "Claim request_id reused with a different owner, scope or lease policy",
                ));
            }
            let mut value: Value =
                serde_json::from_str(&response).map_err(|_| IngestError::Storage)?;
            value["replayed"] = json!(true);
            return Ok(ServiceReply::success(value));
        }
    }
    storage(tx.execute("UPDATE agent_work SET stage='recovery_required',lease=NULL,owner=NULL,busy=0,last_error='Ambiguous interrupted write: inspect current artifacts before retry' WHERE busy=1 AND expires<?1",[at]))?;
    storage(tx.execute("UPDATE agent_work SET lease=NULL,owner=NULL,viewed=NULL,preview=NULL WHERE busy=0 AND expires<?1",[at]))?;
    let mut q=storage(tx.prepare("SELECT w.id FROM agent_work w JOIN agent_jobs j ON j.id=w.job_id WHERE j.state='running' AND w.stage IN ('triage','crop','final','annotation') AND w.lease IS NULL AND (?1=0 OR w.job_id=?1) AND NOT EXISTS(SELECT 1 FROM agent_work active WHERE active.media_id=w.media_id AND active.lease IS NOT NULL) ORDER BY w.id LIMIT ?2"))?;
    let ids: Vec<i64> = storage(
        q.query_map(params![input["job_id"].as_i64().unwrap_or(0), limit], |r| {
            r.get(0)
        })
        .and_then(|r| r.collect()),
    )?;
    drop(q);
    let mut items = Vec::new();
    for id in ids {
        let changed=tx.execute("UPDATE agent_work SET lease=lower(hex(randomblob(32))),owner=?2,expires=?3,generation=generation+1,viewed=NULL,preview=NULL WHERE id=?1 AND lease IS NULL",params![id,owner,at+ttl]);
        match changed {
            Ok(1) => {}
            Ok(_) => continue,
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                continue
            }
            Err(_) => return Err(IngestError::Storage),
        }
        let item=storage(tx.query_row("SELECT job_id,media_id,sha,stage,lease,generation,expires FROM agent_work WHERE id=?1",[id],|r|Ok(json!({"work_id":id,"job_id":r.get::<_,i64>(0)?,"media_id":r.get::<_,i64>(1)?,"expected_sha256":r.get::<_,String>(2)?,"stage":r.get::<_,String>(3)?,"lease_token":r.get::<_,String>(4)?,"generation":r.get::<_,i64>(5)?,"expires":r.get::<_,i64>(6)?}))))?;
        items.push(item);
    }
    let next = if items.is_empty() {
        if input["job_id"].as_i64().is_some() {
            "job_status"
        } else {
            "jobs_list"
        }
    } else {
        "work_view"
    };
    let mut response = json!({"items":items,"next":{"operation":next,"job_id":input["job_id"],"hint":if next=="work_view"{"Load the full current image for each leased item before deciding."}else{"No claimable work. Check preflight, completion, pause and existing leases; run jobs_tick only when local preflight is pending. Do not busy-poll. A cached empty claim needs a new request_id after readiness changes."}}});
    if let Some(request) = request {
        response["claim_request_id"] = json!(request);
        response["replayed"] = json!(false);
        storage(tx.execute(
            "INSERT INTO agent_claims VALUES(?1,?2,?3)",
            params![request, fingerprint, response.to_string()],
        ))?;
    }
    storage(tx.commit())?;
    let mut q = storage(
        repo.connection
            .prepare("SELECT DISTINCT job_id FROM agent_work WHERE stage='recovery_required'"),
    )?;
    let expired: Vec<i64> = storage(q.query_map([], |r| r.get(0)).and_then(|r| r.collect()))?;
    drop(q);
    for job in expired {
        finish(repo, job)?;
    }
    Ok(ServiceReply::success(response))
}

type WorkRecord = (
    i64,
    i64,
    String,
    String,
    Option<String>,
    i64,
    i64,
    i64,
    Option<String>,
    Option<String>,
    String,
    String,
);

fn work(
    service: &AgentService,
    repo: &mut SqliteCatalog,
    action: &str,
    input: Value,
) -> Result<ServiceReply, IngestError> {
    let id = number(&input, "work_id")?;
    let token = text(&input, "lease_token")?;
    let generation = number(&input, "generation")?;
    let at = now();
    let row:WorkRecord=storage(repo.connection.query_row("SELECT w.job_id,w.media_id,w.sha,w.stage,w.lease,w.generation,w.expires,w.busy,w.viewed,w.preview,j.options,j.state FROM agent_work w JOIN agent_jobs j ON j.id=w.job_id WHERE w.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?))))?;
    let (
        job_id,
        media_id,
        sha,
        stage,
        lease,
        gen,
        expires,
        busy,
        viewed,
        preview,
        plan_raw,
        job_state,
    ) = row;
    let request = input["request_id"].as_str();
    let proof = digest(&json!([token, generation]));
    let fingerprint = digest(
        &json!({"action":action,"payload":input["payload"],"expected_sha256":input["expected_sha256"]}),
    );
    if let Some(request) = request {
        if let Some((fp,response,state,receipt_proof))=storage(repo.connection.query_row("SELECT fingerprint,response,state,lease_proof FROM agent_receipts WHERE work_id=?1 AND request_id=?2",params![id,request],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional())?{
            if proof!=receipt_proof{return Err(error("lease_lost","Receipt belongs to another lease"));}
            if fp!=fingerprint{return Err(error("idempotency_conflict","Use a new request_id for a correction"));}
            if state=="complete"{let metadata:Value=serde_json::from_str(&response.unwrap()).map_err(|_|IngestError::Storage)?;let is_error=metadata["status"]=="error"||metadata.get("error").is_some();return Ok(ServiceReply{metadata,is_error,image:None});}
            if state=="intent"{return Err(error("recovery_required","Previous write outcome is ambiguous; never blindly replay"));}
        }
    }
    if lease.as_deref() != Some(token)
        || gen != generation
        || expires <= at
        || busy != 0
        || job_state != "running"
    {
        return Err(error(
            "lease_lost",
            "Lease expired, fenced, busy or job paused/cancelled",
        ));
    }
    if action == "heartbeat" {
        let changed=storage(repo.connection.execute("UPDATE agent_work SET expires=?5 WHERE id=?1 AND lease=?2 AND generation=?3 AND expires>?4 AND EXISTS(SELECT 1 FROM agent_jobs WHERE id=job_id AND state='running')",params![id,token,generation,now(),now()+600]))?;
        if changed != 1 {
            return Err(error("lease_lost", "Ownership changed before heartbeat"));
        }
        return Ok(ServiceReply::success(json!({"expires":at+600})));
    }
    if action == "release" {
        let changed=storage(repo.connection.execute("UPDATE agent_work SET lease=NULL,owner=NULL,viewed=NULL,preview=NULL WHERE id=?1 AND lease=?2 AND generation=?3 AND expires>?4 AND EXISTS(SELECT 1 FROM agent_jobs WHERE id=job_id AND state='running')",params![id,token,generation,now()]))?;
        if changed != 1 {
            return Err(error("lease_lost", "Ownership changed before release"));
        }
        return Ok(ServiceReply::success(json!({"released":true})));
    }
    let media = repo
        .media_by_id(media_id)?
        .ok_or(IngestError::InvalidMedia)?;
    if media.sha256 != sha {
        return Err(error(
            "revision_conflict",
            "Image changed; retry only after reviewing current image",
        ));
    }
    let plan: Value = serde_json::from_str(&plan_raw).map_err(|_| IngestError::Storage)?;
    let settings = Settings {
        project_root: service.settings.project_root.clone(),
        library_root: service.settings.library_root.clone(),
        writes: true,
        permit: Some((id, token.to_owned(), generation)),
        seen: Default::default(),
        previews: Default::default(),
        preview_cache: service.settings.preview_cache.clone(),
    };
    if viewed.as_deref() == Some(&sha) {
        settings.seen.lock().unwrap().insert(sha.clone());
    }
    let mut payload = input["payload"].clone();
    if !payload.is_object() {
        payload = json!({});
    }
    payload["media_id"] = json!(media_id);
    payload["expected_sha256"] = json!(sha);
    if action == "view" {
        payload.as_object_mut().unwrap().remove("expected_sha256");
        let args: ViewArgs = serde_json::from_value(payload.clone()).map_err(|_| {
            error(
                "invalid_arguments",
                "view payload: original and optional bbox only",
            )
        })?;
        let full = !args.original && args.bbox_1000_xyxy.is_none();
        let mut result = tools::view(repo, &settings, args)?;
        if full {
            let changed=storage(repo.connection.execute("UPDATE agent_work SET viewed=?5,expires=?6 WHERE id=?1 AND lease=?2 AND generation=?3 AND expires>?4 AND EXISTS(SELECT 1 FROM agent_jobs WHERE id=job_id AND state='running')",params![id,token,generation,now(),sha,now()+600]))?;
            if changed != 1 {
                return Err(error(
                    "lease_lost",
                    "Ownership changed while image was being delivered",
                ));
            }
        }
        route(
            &mut result,
            id,
            &stage,
            match stage.as_str() {
                "triage" | "final" => "work_assess",
                "crop" => "work_preview",
                _ => "work_annotate",
            },
        );
        return Ok(result);
    }
    if viewed.as_deref() != Some(&sha) {
        return Err(error(
            "inspection_required",
            "Use work view on the full current image first",
        ));
    }
    if input["expected_sha256"].as_str() != Some(&sha) {
        return Err(error(
            "revision_conflict",
            "Submit the inspected expected_sha256",
        ));
    }
    if action == "preview" {
        if stage != "crop" {
            return Err(error(
                "wrong_stage",
                "Crop previews require a crop candidate",
            ));
        }
        payload["apply"] = json!(false);
        let args: CropArgs = serde_json::from_value(payload.clone()).map_err(|_| {
            error(
                "invalid_arguments",
                "crop payload requires bbox, reason and model",
            )
        })?;
        let mut result = tools::crop(repo, &settings, args)?;
        payload["preview_sha256"] = result.metadata["preview_sha256"].clone();
        let changed=storage(repo.connection.execute("UPDATE agent_work SET preview=?5,expires=?6 WHERE id=?1 AND lease=?2 AND generation=?3 AND expires>?4 AND EXISTS(SELECT 1 FROM agent_jobs WHERE id=job_id AND state='running')",params![id,token,generation,now(),payload.to_string(),now()+600]))?;
        if changed != 1 {
            return Err(error("lease_lost", "Ownership changed during preview"));
        }
        route(&mut result, id, &stage, "work_apply");
        return Ok(result);
    }
    if let Some(preview) = preview.as_ref() {
        let p: Value = serde_json::from_str(preview).map_err(|_| IngestError::Storage)?;
        let bbox: [u32; 4] = serde_json::from_value(p["bbox_1000_xyxy"].clone())
            .map_err(|_| IngestError::Storage)?;
        settings.previews.lock().unwrap().insert(
            (media_id, sha.clone(), bbox),
            (
                p["ratio"].as_str().unwrap_or("").to_owned(),
                p["preview_sha256"].as_str().unwrap_or("").to_owned(),
            ),
        );
    }
    let allowed = match action {
        "apply" => stage == "crop",
        "assess" => matches!(stage.as_str(), "triage" | "crop" | "final"),
        "annotate" => stage == "annotation",
        _ => false,
    };
    if !allowed {
        return Err(error(
            "wrong_stage",
            "Operation not allowed by the server-owned task stage",
        ));
    }
    let request = text(&input, "request_id")?;
    let tx = storage(
        repo.connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
    )?;
    let claimed=storage(tx.execute("UPDATE agent_work SET busy=1,expires=?4 WHERE id=?1 AND lease=?2 AND generation=?3 AND busy=0 AND expires>?5 AND EXISTS(SELECT 1 FROM agent_jobs WHERE id=agent_work.job_id AND state='running')",params![id,token,generation,at+300,at]))?;
    if claimed != 1 {
        return Err(error("lease_lost", "Ownership changed before commit"));
    }
    storage(tx.execute(
        "INSERT INTO agent_receipts VALUES(?1,?2,?3,NULL,'intent',?4)",
        params![id, request, fingerprint, proof],
    ))?;
    storage(tx.commit())?;
    let outcome: Result<(ServiceReply, String, String), IngestError> = (|| match action {
        "apply" => {
            let previous: Value = serde_json::from_str(
                preview
                    .as_ref()
                    .ok_or_else(|| error("preview_required", "Preview the exact crop first"))?,
            )
            .map_err(|_| IngestError::Storage)?;
            if previous["bbox_1000_xyxy"] != payload["bbox_1000_xyxy"]
                || previous["ratio"] != payload["ratio"]
                || previous["preview_sha256"]
                    .as_str()
                    .is_none_or(|sha| Some(sha) != payload["preview_sha256"].as_str())
            {
                return Err(error(
                    "preview_required",
                    "Echo the exact inspected proposal's bbox, ratio and preview_sha256 before applying",
                ));
            }
            payload["apply"] = json!(true);
            let args: CropArgs = serde_json::from_value(payload)
                .map_err(|e| error("invalid_arguments", &e.to_string()))?;
            let result = tools::crop(repo, &settings, args)?;
            let updated = repo
                .media_by_id(media_id)?
                .ok_or(IngestError::InvalidMedia)?;
            Ok((result, "final".into(), updated.sha256))
        }
        "assess" => {
            let mut args: AssessArgs = serde_json::from_value(payload)
                .map_err(|e| error("invalid_arguments", &e.to_string()))?;
            args.stage = if stage == "triage" && args.crop_candidate {
                AssessmentStage::Preliminary
            } else {
                AssessmentStage::Final
            };
            if stage != "triage" {
                args.crop_candidate = false;
            }
            if matches!(args.stage, AssessmentStage::Final)
                && matches!(args.decision, Decision::Keep)
            {
                let elements=args.retained_elements.as_ref().ok_or_else(||error("retained_elements_required","Inspect final pixels and report retained_elements, including [] when none; do not infer absence from source tags"))?;
                if !elements.is_empty()
                    && !plan["outputs"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("regions"))
                {
                    return Err(error("regions_output_required","Retained text/watermarks/stickers require a plan with regions output; preserve the image and submit a reviewed new plan"));
                }
            }
            let result = tools::assess(repo, &settings, args)?;
            let next = if result.metadata["stage"] == "preliminary" {
                "crop"
            } else {
                match result.metadata["decision"].as_str() {
                    Some("keep") => "annotation",
                    Some("reject") => "review",
                    _ => "deferred",
                }
            };
            Ok((result, next.into(), sha.clone()))
        }
        "annotate" => {
            let outputs = plan["outputs"].as_array().unwrap();
            for (field, target) in [
                ("caption", "caption"),
                ("tags", "tags"),
                ("text_regions", "regions"),
                ("visual_regions", "regions"),
                ("special_elements", "regions"),
            ] {
                if !payload[field].is_null() && !outputs.contains(&json!(target)) {
                    return Err(error("output_not_requested", field));
                }
            }
            if !payload["review_reasons"].is_null() {
                return Err(error(
                    "output_not_requested",
                    "Use assessment to route review in this job",
                ));
            }
            let mut args: AnnotationArgs = serde_json::from_value(payload)
                .map_err(|e| error("invalid_arguments", &e.to_string()))?;
            args.require_final_assessment = plan["mode"] == "curate";
            args.overwrite = plan["overwrite"].as_bool().unwrap_or(false)
                || plan["overwrite"].as_i64() == Some(1);
            let mut result = tools::annotate(repo, &settings, args)?;
            let required: Vec<&str> = outputs
                .iter()
                .filter_map(Value::as_str)
                .filter(|s| *s != "regions")
                .collect();
            let regions_done = !outputs.contains(&json!("regions"))
                || result.metadata["annotation_details"]["image_sha256"] == sha;
            let mut done = regions_done;
            let mut blocked = false;
            for kind in required {
                let complete = output_complete(repo, &media, kind)?;
                done &= complete;
                blocked |= !complete
                    && result.metadata["preserved"]
                        .as_array()
                        .is_some_and(|a| a.contains(&json!(kind)));
            }
            if blocked {
                result.metadata["warning"]=json!("Protected text is stale or manually changed. Preserved bytes; review before authorizing a new overwrite plan.");
            }
            Ok((
                result,
                if done {
                    "completed"
                } else if blocked {
                    "deferred"
                } else {
                    "annotation"
                }
                .into(),
                sha.clone(),
            ))
        }
        _ => unreachable!(),
    })();
    match outcome {
        Ok((mut reply, next, new_sha)) => {
            let operation = match next.as_str() {
                "final" => "work_view",
                "crop" => "work_preview",
                "annotation" => "work_annotate",
                _ => "work_claim",
            };
            route(&mut reply, id, &next, operation);
            let current = repo
                .media_by_id(media_id)?
                .ok_or(IngestError::InvalidMedia)?;
            let revision = annotation_revision(repo, &current)?;
            let summary = json!({"stage":next,"status":reply.metadata["status"],"saved":reply.metadata["saved"],"preserved":reply.metadata["preserved"],"warning":reply.metadata["warning"],"bucket_plan":reply.metadata["bucket_plan"],"sha256":new_sha});
            let tx = storage(
                repo.connection
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
            )?;
            let changed=storage(tx.execute("UPDATE agent_work SET stage=CASE WHEN (SELECT state FROM agent_jobs WHERE id=job_id)='cancelled' THEN 'cancelled' ELSE ?2 END,sha=?3,busy=0,viewed=CASE WHEN sha=?3 THEN viewed ELSE NULL END,preview=CASE WHEN sha=?3 THEN preview ELSE NULL END,lease=CASE WHEN ?4 OR (SELECT state FROM agent_jobs WHERE id=job_id)<>'running' THEN NULL ELSE lease END,expires=?5 WHERE id=?1 AND lease=?6 AND generation=?7 AND busy=1",params![id,next,new_sha,matches!(next.as_str(),"completed"|"review"|"deferred"),at+600,token,generation]))?;
            if changed != 1 {
                return Err(error("recovery_required","Effect may have completed after ownership changed; inspect current artifacts before a new plan"));
            }
            storage(tx.execute(
                "UPDATE agent_work SET revision=?2,result=?3 WHERE id=?1",
                params![id, revision, summary.to_string()],
            ))?;
            storage(tx.execute("UPDATE agent_receipts SET state='complete',response=?3 WHERE work_id=?1 AND request_id=?2",params![id,request,reply.metadata.to_string()]))?;
            storage(tx.commit())?;
            finish(repo, job_id)?;
            Ok(reply)
        }
        Err(e) => {
            // The conservative failure path never blindly repeats a possibly
            // committed image/text write. Explicit receipts preserve diagnosis.
            let reply = tools::error_result(e.clone());
            let code = reply.metadata["code"].as_str().unwrap_or("");
            let safe = matches!(
                code,
                "invalid_arguments"
                    | "preview_required"
                    | "output_not_requested"
                    | "invalid_caption"
                    | "invalid_tags"
                    | "invalid_provenance"
                    | "invalid_text_regions"
                    | "invalid_visual_regions"
                    | "invalid_special_elements"
                    | "invalid_review_reasons"
                    | "stale_annotation_details"
                    | "final_assessment_required"
                    | "crop_pending"
                    | "inspection_required"
                    | "invalid_assessment_stage"
                    | "crop_first_required"
                    | "crop_preview_required"
                    | "no_pending_crop"
                    | "bucket_unavailable"
                    | "crop_ratio_required"
                    | "retained_elements_required"
                    | "invalid_retained_elements"
                    | "retained_elements_bbox_required"
                    | "tag_coverage_incomplete"
            );
            let conflict = matches!(
                code,
                "annotation_conflict"
                    | "stale_image"
                    | "human_review_required"
                    | "regions_output_required"
            );
            if safe || conflict {
                storage(repo.connection.execute(
                    "UPDATE agent_work SET busy=0,last_error=?2 WHERE id=?1",
                    params![id, e.to_string()],
                ))?;
                storage(repo.connection.execute("UPDATE agent_receipts SET state='complete',response=?3 WHERE work_id=?1 AND request_id=?2",params![id,request,reply.metadata.to_string()]))?;
                if conflict {
                    storage(repo.connection.execute(
                        "UPDATE agent_work SET stage=?2,lease=NULL,owner=NULL WHERE id=?1",
                        params![
                            id,
                            if matches!(code, "human_review_required" | "regions_output_required") {
                                "deferred"
                            } else {
                                "failed"
                            }
                        ],
                    ))?;
                }
            } else {
                storage(repo.connection.execute("UPDATE agent_work SET busy=0,stage='recovery_required',lease=NULL,owner=NULL,last_error=?2 WHERE id=?1",params![id,e.to_string()]))?;
            }
            finish(repo, job_id)?;
            Err(e)
        }
    }
}

pub fn tick(service: &AgentService, repo: &mut SqliteCatalog) -> Result<Value, IngestError> {
    let tx = storage(
        repo.connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
    )?;
    // A dead local worker may have partially exported files. Preserve diagnosis
    // and require reconciliation rather than duplicating its side effects.
    let mut q =
        storage(tx.prepare("SELECT DISTINCT job_id FROM agent_work WHERE busy=1 AND expires<?1"))?;
    let expired: Vec<i64> = storage(q.query_map([now()], |r| r.get(0)).and_then(|r| r.collect()))?;
    drop(q);
    storage(tx.execute("UPDATE agent_work SET stage='recovery_required',lease=NULL,owner=NULL,busy=0,last_error='Interrupted worker; inspect artifacts before resubmitting' WHERE busy=1 AND expires<?1",[now()]))?;
    let row:Option<(i64,i64,i64,String,String)>=storage(tx.query_row("SELECT w.id,w.job_id,w.media_id,j.options,w.sha FROM agent_work w JOIN agent_jobs j ON j.id=w.job_id WHERE w.stage='local' AND j.state='running' ORDER BY w.id LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional())?;
    let Some((id, job, media, raw, expected)) = row else {
        storage(tx.commit())?;
        for job in expired {
            finish(repo, job)?;
        }
        return super::downloads::tick(service, repo);
    };
    let mut plan: Value = serde_json::from_str(&raw).map_err(|_| IngestError::Storage)?;
    let export = plan["mode"] == "export";
    if export {
        let mut q = storage(
            tx.prepare("SELECT media_id,sha,id FROM agent_work WHERE job_id=?1 ORDER BY id"),
        )?;
        let items: Vec<Value> = storage(
            q.query_map([job], |r| {
                Ok(json!({"media_id":r.get::<_,i64>(0)?,"sha256":r.get::<_,String>(1)?,"work_id":r.get::<_,i64>(2)?}))
            })
            .and_then(|r| r.collect()),
        )?;
        plan["items"] = json!(items);
        storage(tx.execute("UPDATE agent_work SET stage='local_running',busy=1,expires=?2 WHERE job_id=?1 AND stage='local'",params![job,now()+300]))?;
    } else {
        storage(tx.execute("UPDATE agent_work SET stage='local_running',busy=1,expires=?2 WHERE id=?1 AND stage='local'",params![id,now()+300]))?;
    }
    storage(tx.commit())?;
    for previous in expired {
        finish(repo, previous)?;
    }
    let cancelled = AtomicBool::new(false);
    let done = AtomicBool::new(false);
    let outcome = std::thread::scope(|scope| {
        if export {
            scope.spawn(|| {
            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok(db)=service.connection(false) {
                    let state:Result<String,_>=db.connection.query_row("SELECT state FROM agent_jobs WHERE id=?1",[job],|r|r.get(0));
                    if state.as_deref()!=Ok("running") {cancelled.store(true,std::sync::atomic::Ordering::Relaxed);}
                    let _=db.connection.execute("UPDATE agent_work SET expires=?2 WHERE job_id=?1 AND busy=1 AND stage='local_running'",params![job,now()+300]);
                    let _=db.connection.execute("UPDATE agent_daemon SET heartbeat=?2 WHERE id=1 AND pid=?1",params![std::process::id(),now()]);
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        });
        }
        let result = (|| -> Result<String, IngestError> {
            if export {
                let ids: Vec<i64> = plan["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|i| i["media_id"].as_i64().unwrap())
                    .collect();
                for item in plan["items"].as_array().unwrap() {
                    tools::media(
                        repo,
                        &service.settings,
                        item["media_id"].as_i64().unwrap(),
                        item["sha256"].as_str(),
                    )?;
                }
                let mut output = repo.new_workflow("export", ids)?;
                output.mode = plan["text_kind"].as_str().unwrap().into();
                output.export_layout = plan["export_layout"].as_str().unwrap_or("flat").into();
                output.dismissed = true;
                crate::export::run(
                    repo,
                    &mut output,
                    Path::new(plan["destination"].as_str().unwrap()),
                    &cancelled,
                    |_| {},
                )?;
                if output.state != "completed" {
                    return Err(error(
                        "export_incomplete",
                        "Export stopped or contains deferred items; inspect export history",
                    ));
                }
                let tx = storage(
                    repo.connection
                        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
                )?;
                for item in plan["items"].as_array().unwrap() {
                    let exported = output
                        .accepted
                        .contains(&item["media_id"].as_i64().unwrap());
                    storage(tx.execute("UPDATE agent_work SET stage=CASE WHEN (SELECT state FROM agent_jobs WHERE id=job_id)='cancelled' THEN 'cancelled' ELSE ?2 END,busy=0,result=?3 WHERE id=?1 AND stage='local_running'",params![item["work_id"].as_i64().unwrap(),if exported{"completed"}else{"deferred"},json!({"exported":exported,"export_job_id":output.id,"reason":if exported{"exported or existing managed output preserved"}else{"not approved or no usable selected TXT"}}).to_string()]))?;
                }
                storage(tx.commit())?;
                return Ok("completed".into());
            }
            let m = tools::media(repo, &service.settings, media, None)?;
            if m.sha256 != expected {
                return Err(error(
                    "revision_conflict",
                    "Snapshot changed before preflight",
                ));
            }
            if plan["mode"] == "annotate" && plan["overwrite"] != true {
                let mut complete = true;
                for output in plan["outputs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                {
                    complete &= if output == "regions" {
                        tools::annotation_details(repo, m.id)?
                            .image_sha256
                            .as_deref()
                            == Some(&m.sha256)
                    } else {
                        output_complete(repo, &m, output)?
                    };
                }
                if complete {
                    return Ok("completed".into());
                }
            }
            let choice = match bdm_domain::training::best_bucket(
                m.dimensions().0,
                m.dimensions().1,
                "original",
            ) {
                Ok(choice) => choice,
                Err(_) if plan["mode"] == "annotate" => return Ok("annotation".into()),
                Err(message) => return Err(error("bucket_unavailable", message)),
            };
            storage(repo.connection.execute(
                "UPDATE agent_work SET result=?2 WHERE id=?1",
                params![id, json!({"bucket_plan":choice}).to_string()],
            ))?;
            repo.audit(
                "agent_bucket_plan",
                &json!({"id":media,"plan":choice}).to_string(),
            )?;
            Ok(if plan["mode"] == "bucket" {
                "completed"
            } else if plan["mode"] == "curate" {
                "triage"
            } else {
                "annotation"
            }
            .into())
        })();
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        result
    });
    let (stage, err) = match outcome {
        Ok(stage) => (stage, None),
        Err(_) if export && cancelled.load(std::sync::atomic::Ordering::Relaxed) => {
            ("local".into(), None)
        }
        Err(e) => ("failed".into(), Some(e.to_string())),
    };
    // Respect a concurrent pause/cancel. A paused local operation is replanned
    // only after an explicit resume; completed atomic effects are not replayed.
    storage(repo.connection.execute("UPDATE agent_work SET stage=CASE WHEN (SELECT state FROM agent_jobs WHERE id=job_id)='cancelled' THEN 'cancelled' ELSE ?3 END,busy=0,last_error=?4 WHERE job_id=?1 AND (id=?2 OR ?5) AND stage='local_running'",params![job,id,stage,err,export]))?;
    finish(repo, job)?;
    Ok(
        json!({"processed":if export {plan["scope_count"].as_u64().unwrap()}else{1},"job_id":job,"media_id":media,"stage":stage,"error":err}),
    )
}

pub(super) fn dispatch(
    service: &AgentService,
    repo: &mut SqliteCatalog,
    operation: &str,
    input: Value,
) -> Result<ServiceReply, IngestError> {
    if matches!(operation, "doctor" | "status") {
        use bdm_ports::SourceCredentialStore;
        let (dan, gel) = super::downloads::credentials(&service.settings.project_root);
        let credential_state = |store: crate::credentials::NativeCredentialStore| match store.load()
        {
            Ok(Some(_)) => "configured",
            Ok(None) => "not_configured",
            Err(_) => "store_unavailable",
        };
        let count: i64 = storage(repo.connection.query_row(
            "SELECT count(*) FROM media",
            [],
            |r| r.get(0),
        ))?;
        let daemon: Option<(i64, i64)> = storage(
            repo.connection
                .query_row(
                    "SELECT pid,heartbeat FROM agent_daemon WHERE id=1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional(),
        )?;
        let runtime = if input["probe_runtime"].as_bool() == Some(true) {
            service.runtime_probe()
        } else {
            json!({"state":"not_probed","hint":"doctor with probe_runtime=true checks Python and modules without loading models or using the network"})
        };
        return Ok(ServiceReply::success(
            json!({"protocol":"bdm-agent-v3","catalog_readable":true,"media_count":count,"library_exists":service.settings.library_root.is_dir(),"daemon":daemon.map(|(pid,time)|json!({"pid":pid,"alive":now()-time<10})),"sources":{"danbooru":credential_state(dan),"gelbooru":credential_state(gel)},"models":{"dinov3_weights_present":service.settings.project_root.join("models/dinov3/model.safetensors").is_file(),"runtime":runtime,"weight_integrity":"not_checked"},"network_probed":false,"secrets_returned":false}),
        ));
    }
    if operation == "job_plan" {
        let mut value = plan(repo, &input)?;
        let sha = digest(&value);
        value["plan_sha256"] = json!(sha);
        let truncated =
            value["items"].as_array().unwrap().len() > 16 && input["include_items"] != true;
        if truncated {
            value["items"].as_array_mut().unwrap().truncate(16);
        }
        value["items_truncated"] = json!(truncated);
        return Ok(ServiceReply::success(value));
    }
    if operation == "job_status" {
        return Ok(ServiceReply::success(status_page(
            repo,
            number(&input, "job_id")?,
            input["after_id"].as_i64().unwrap_or(0),
            input["limit"].as_u64().unwrap_or(64).clamp(1, 128),
            input["include_plan"].as_bool().unwrap_or(false),
        )?));
    }
    if operation == "jobs_list" {
        let mut q = storage(
            repo.connection
                .prepare("SELECT id,state FROM agent_jobs WHERE id>?1 ORDER BY id LIMIT ?2"),
        )?;
        let jobs: Vec<Value> = storage(
            q.query_map(
                params![
                    input["after_id"].as_i64().unwrap_or(0),
                    input["limit"].as_u64().unwrap_or(20).clamp(1, 64)
                ],
                |r| Ok(json!({"job_id":r.get::<_,i64>(0)?,"state":r.get::<_,String>(1)?})),
            )
            .and_then(|r| r.collect()),
        )?;
        return Ok(ServiceReply::success(json!({"jobs":jobs})));
    }
    writable(service)?;
    match operation {
        "job_submit" => submit(repo, input),
        "job_control" => Ok(ServiceReply::success(control(
            repo,
            text(&input, "action")?,
            &input,
        )?)),
        "jobs_tick" => Ok(ServiceReply::success(tick(service, repo)?)),
        "work_claim" => claim(repo, &input),
        "daemon_stop" => {
            storage(
                repo.connection
                    .execute("UPDATE agent_daemon SET stop=1 WHERE id=1", []),
            )?;
            Ok(ServiceReply::success(json!({"stop_requested":true})))
        }
        op if op.starts_with("work_") => work(service, repo, &op[5..], input),
        _ => Err(error("unknown_operation", operation)),
    }
}

pub fn serve_daemon(service: AgentService) -> Result<(), Box<dyn std::error::Error>> {
    use fs2::FileExt;
    if !service.settings.writes {
        return Err("Daemon requires --allow-writes".into());
    }
    let path = service.catalog.with_extension("agent-daemon.lock");
    let owner = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    owner
        .try_lock_exclusive()
        .map_err(|_| "A daemon already owns this catalog")?;
    let mut repo = service.connection(false)?;
    repo.connection.execute("INSERT INTO agent_daemon VALUES(1,?1,?2,0) ON CONFLICT(id) DO UPDATE SET pid=excluded.pid,heartbeat=excluded.heartbeat,stop=0",params![std::process::id(),now()])?;
    loop {
        let stop: bool =
            repo.connection
                .query_row("SELECT stop FROM agent_daemon WHERE id=1", [], |r| r.get(0))?;
        if stop {
            break;
        }
        repo.connection
            .execute("UPDATE agent_daemon SET heartbeat=?1 WHERE id=1", [now()])?;
        if tick(&service, &mut repo)?["processed"] == 0 {
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }
    repo.connection
        .execute("UPDATE agent_daemon SET heartbeat=0 WHERE id=1", [])?;
    Ok(())
}
