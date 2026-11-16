//! Queue adapter around the same ingestion use case used by the desktop.
use super::{
    jobs::now,
    service::{AgentService, ServiceReply},
};
use crate::{catalog::SqliteCatalog, credentials::NativeCredentialStore, ingest::storage};
use bdm_domain::{
    ingest::IngestJob,
    sources::{DownloadSource, SourceProvider},
};
use bdm_ports::{IngestError, IngestRepository, SourceCredentialStore};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub(crate) const SCHEMA:&str="CREATE TABLE IF NOT EXISTS agent_downloads(id INTEGER PRIMARY KEY REFERENCES ingest_jobs(id),request_id TEXT UNIQUE NOT NULL,fingerprint TEXT NOT NULL,state TEXT NOT NULL,expires INTEGER NOT NULL DEFAULT 0);";

pub(super) fn credentials(root: &Path) -> (NativeCredentialStore, NativeCredentialStore) {
    let portable = root.join("portable.flag").is_file();
    (
        if portable {
            NativeCredentialStore::danbooru_portable(root)
        } else {
            NativeCredentialStore::danbooru()
        },
        NativeCredentialStore::gelbooru(portable.then_some(root)),
    )
}
fn fail(message: &str) -> IngestError {
    IngestError::StorageDetails(json!({"code":"invalid_download","message":message,"next":{"action":"check_download_arguments"}}).to_string())
}

pub(super) fn dispatch(
    service: &AgentService,
    repo: &mut SqliteCatalog,
    operation: &str,
    input: Value,
) -> Result<ServiceReply, IngestError> {
    let id = input["download_id"].as_i64().unwrap_or(0);
    if operation == "download_status" {
        let raw:String=storage(repo.connection.query_row("SELECT i.metadata FROM ingest_jobs i JOIN agent_downloads a ON i.id=a.id WHERE a.id=?1",[id],|r|r.get(0)))?;
        let mut value: Value = serde_json::from_str(&raw).map_err(|_| IngestError::Storage)?;
        // Persisted source post URLs may contain transient signed parameters.
        // Only publish counters and the sanitized provider error in this summary.
        value.as_object_mut().unwrap().remove("failed_posts");
        value.as_object_mut().unwrap().remove("root");
        let job: IngestJob = serde_json::from_str(&raw).map_err(|_| IngestError::Storage)?;
        value["source_label"] = json!(job.source_label());
        return Ok(ServiceReply::success(value));
    }
    if !service.settings.writes {
        return Err(fail("Restart with --allow-writes"));
    }
    if operation == "download_submit" {
        let request = input["request_id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
            .ok_or_else(|| fail("request_id required"))?;
        let collection = input["collection_id"]
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or_else(|| fail("Existing collection_id required"))?;
        let side = input["minimum_short_side"].as_u64().unwrap_or(800);
        if side > u64::from(u32::MAX) || !bdm_domain::ingest::valid_minimum_short_side(side as u32)
        {
            return Err(fail("Invalid minimum_short_side"));
        }
        let source = input["source"].as_str().unwrap_or("automatic");
        let mode = match source {
            "automatic" => DownloadSource::Automatic,
            "danbooru" => DownloadSource::Danbooru,
            "safebooru" | "safedanbooru" => DownloadSource::Safebooru,
            "gelbooru" => DownloadSource::Gelbooru,
            _ => return Err(fail("source: automatic/danbooru/safebooru/gelbooru")),
        };
        let source = if mode == DownloadSource::Safebooru {
            "safebooru"
        } else {
            source
        };
        let (tag, provider): (String, String) = storage(repo.connection.query_row(
            "SELECT tag,provider FROM collections WHERE id=?1",
            [collection],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ))?;
        let provider = mode.initial_provider(if provider == "gelbooru" {
            SourceProvider::Gelbooru
        } else {
            SourceProvider::Danbooru
        });
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(json!([collection, side, source]).to_string().as_bytes())
        );
        let tx = storage(
            repo.connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
        )?;
        if let Some((id, previous)) = storage(
            tx.query_row(
                "SELECT id,fingerprint FROM agent_downloads WHERE request_id=?1",
                [request],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional(),
        )? {
            if previous != fingerprint {
                return Err(fail("request_id reused with changed arguments"));
            }
            return Ok(ServiceReply::success(
                json!({"download_id":id,"replayed":true}),
            ));
        }
        storage(tx.execute(
            "INSERT INTO ingest_jobs(collection_id,metadata) VALUES(?1,'{}')",
            [collection],
        ))?;
        let id = tx.last_insert_rowid();
        let job = IngestJob {
            id,
            collection_id: collection,
            tag,
            root: service.settings.library_root.clone(),
            minimum_short_side: side as u32,
            flatten_transparency: bdm_domain::ingest::default_flatten_transparency(),
            provider,
            source_mode: mode,
            primary_before_id: None,
            fallback_reason: None,
            dismissed: false,
            before_id: None,
            state: "queued".into(),
            progress: Default::default(),
            error: None,
            failed_posts: Vec::new(),
        };
        storage(tx.execute(
            "UPDATE ingest_jobs SET metadata=?2 WHERE id=?1",
            params![id, serde_json::to_string(&job).unwrap()],
        ))?;
        storage(tx.execute(
            "INSERT INTO agent_downloads VALUES(?1,?2,?3,'queued',0)",
            params![id, request, fingerprint],
        ))?;
        storage(tx.commit())?;
        repo.audit(
            "agent_job_event",
            &json!({"download_id":id,"state":"queued"}).to_string(),
        )?;
        return Ok(ServiceReply::success(
            json!({"download_id":id,"state":"queued","next":{"operation":"jobs_tick","hint":"Daemon runs the existing downloader; network requests start only during execution."}}),
        ));
    }
    if operation == "download_control" {
        return Ok(ServiceReply::success(control(
            repo,
            id,
            input["action"].as_str().unwrap_or(""),
        )?));
    }

    Err(fail("Unknown download operation"))
}

pub(super) fn tick(service: &AgentService, repo: &mut SqliteCatalog) -> Result<Value, IngestError> {
    let tx = storage(
        repo.connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
    )?;
    storage(tx.execute("UPDATE ingest_jobs SET metadata=json_set(metadata,'$.state','paused','$.error','Local worker stopped; resume continues the saved cursor') WHERE id IN (SELECT id FROM agent_downloads WHERE state IN ('running','pausing') AND expires<?1)",[now()]))?;
    storage(tx.execute("UPDATE agent_downloads SET state='paused' WHERE state IN ('running','pausing') AND expires<?1",[now()]))?;
    let row:Option<(i64,String)>=storage(tx.query_row("SELECT a.id,i.metadata FROM agent_downloads a JOIN ingest_jobs i ON a.id=i.id WHERE a.state='queued' AND NOT EXISTS(SELECT 1 FROM agent_downloads active WHERE active.state IN ('running','pausing')) ORDER BY a.id LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).optional())?;
    let Some((id, raw)) = row else {
        storage(tx.commit())?;
        return Ok(json!({"processed":0}));
    };
    storage(tx.execute(
        "UPDATE agent_downloads SET state='running',expires=?2 WHERE id=?1",
        params![id, now() + 300],
    ))?;
    storage(tx.commit())?;
    let mut job: IngestJob = serde_json::from_str(&raw).map_err(|_| IngestError::Storage)?;
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let done = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        scope.spawn(||{while !done.load(Ordering::Relaxed){
            if let Ok(db)=service.connection(false){
                let state:Result<String,_>=db.connection.query_row("SELECT state FROM agent_downloads WHERE id=?1",[id],|r|r.get(0));
                if state.as_deref()!=Ok("running"){cancel.store(true,Ordering::Relaxed);}
                let _=db.connection.execute("UPDATE agent_downloads SET expires=?2 WHERE id=?1 AND state IN ('running','pausing')",params![id,now()+300]);
                let _=db.connection.execute("UPDATE agent_daemon SET heartbeat=?2 WHERE id=1 AND pid=?1",params![std::process::id(),now()]);
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }});
        let result = {
            let (dan, gel) = credentials(&service.settings.project_root);
            let dan: Box<dyn bdm_ports::MediaProvider> = match dan
                .load()
                .map_err(|_| bdm_ports::CatalogError::Credentials)
                .and_then(|credentials| {
                    crate::danbooru::DanbooruProvider::for_source(credentials, job.source_mode)
                }) {
                Ok(p) => Box::new(p),
                Err(e) => Box::new(crate::providers::UnavailableProvider(e)),
            };
            let gel: Box<dyn bdm_ports::MediaProvider> = match gel
                .load()
                .map_err(|_| bdm_ports::CatalogError::GelbooruCredentials)
                .and_then(crate::gelbooru::GelbooruProvider::new)
            {
                Ok(p) => Box::new(p.with_cancel(cancel.clone())),
                Err(e) => Box::new(crate::providers::UnavailableProvider(e)),
            };
            let mut last = 0;
            bdm_application::ingest::run_with_fallback(
                dan.as_ref(),
                gel.as_ref(),
                repo,
                &mut job,
                &cancel,
                |_| {
                    if now() != last {
                        last = now();
                        if let Ok(db) = service.connection(false) {
                            let _ = db.audit(
                                "agent_job_progress",
                                &json!({"download_id":id}).to_string(),
                            );
                        }
                    }
                },
            )
        };
        done.store(true, Ordering::Relaxed);
        result
    });
    if let Err(e) = &result {
        job.state = "failed".into();
        job.error = Some(e.to_string());
    }
    if cancel.load(Ordering::Relaxed) {
        job.state = "paused".into();
    }
    job.dismissed = matches!(job.state.as_str(), "completed" | "failed" | "deferred");
    repo.save_job(&job)?;
    storage(repo.connection.execute(
        "UPDATE agent_downloads SET state=?2,expires=0 WHERE id=?1",
        params![id, job.state],
    ))?;
    repo.audit(
        "agent_job_event",
        &json!({"download_id":id,"state":job.state}).to_string(),
    )?;
    Ok(json!({"processed":1,"download_id":id,"state":job.state,"error":job.error}))
}

pub(crate) fn managed(repo: &SqliteCatalog, id: i64) -> Result<bool, IngestError> {
    storage(repo.connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_downloads WHERE id=?1)",
        [id],
        |r| r.get(0),
    ))
}
pub(crate) fn control(
    repo: &mut SqliteCatalog,
    id: i64,
    action: &str,
) -> Result<Value, IngestError> {
    let tx = storage(
        repo.connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
    )?;
    let current: String = storage(tx.query_row(
        "SELECT state FROM agent_downloads WHERE id=?1",
        [id],
        |r| r.get(0),
    ))?;
    let state = match (action, current.as_str()) {
        ("pause", "running") => "pausing",
        ("pause", "queued") => "paused",
        ("resume" | "retry", "paused" | "failed" | "deferred") => "queued",
        _ => return Err(fail("Download state does not permit pause/resume/retry")),
    };
    storage(tx.execute(
        "UPDATE agent_downloads SET state=?2 WHERE id=?1",
        params![id, state],
    ))?;
    let raw: String = storage(tx.query_row(
        "SELECT metadata FROM ingest_jobs WHERE id=?1",
        [id],
        |r| r.get(0),
    ))?;
    let mut job: IngestJob = serde_json::from_str(&raw).map_err(|_| IngestError::Storage)?;
    job.state = state.into();
    job.dismissed = false;
    storage(tx.execute(
        "UPDATE ingest_jobs SET metadata=?2 WHERE id=?1",
        params![id, serde_json::to_string(&job).unwrap()],
    ))?;
    storage(tx.commit())?;
    repo.audit(
        "agent_job_event",
        &json!({"download_id":id,"state":state}).to_string(),
    )?;
    Ok(json!({"download_id":id,"state":state}))
}
