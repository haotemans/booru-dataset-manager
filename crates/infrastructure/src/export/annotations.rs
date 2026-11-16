//! One dataset-level annotation index. No extra per-image TXT or image copies.
use super::{atomic_write, file_sha};
use crate::{catalog::SqliteCatalog, ingest::storage, workflows::WorkflowJob};
use bdm_ports::IngestError;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

pub(super) fn write_index(
    repo: &SqliteCatalog,
    job: &WorkflowJob,
    root: &Path,
) -> Result<(), IngestError> {
    if job.accepted.is_empty() {
        return Ok(());
    }
    let lock = repo.media_lock.clone();
    let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
    let path = root.join("annotations.jsonl");
    let receipt = root.join(".bdm-manifests/annotations-index.json");
    let mut entries = BTreeMap::<String, Value>::new();
    if path.exists() {
        let previous: Value = storage(serde_json::from_slice(&storage(fs::read(&receipt))?))?;
        let actual = file_sha(&path)?;
        if previous["schema"] != "bdm-annotation-index-receipt-v1"
            || !["sha256", "pending_sha256"]
                .iter()
                .any(|key| previous[key].as_str() == Some(actual.as_str()))
        {
            return Err(IngestError::Integrity);
        }
        for line in storage(fs::read_to_string(&path))?
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            let record: Value = storage(serde_json::from_str(line))?;
            let key = record["image"]
                .as_str()
                .ok_or(IngestError::Integrity)?
                .to_owned();
            entries.insert(key, record);
        }
    }
    let details = repo.list_annotation_details()?;
    let artifacts = repo.list_artifacts()?;
    // Read audit history once per export, not one whole-table scan per image.
    let mut histories = BTreeMap::<i64, Vec<Value>>::new();
    let mut query=storage(repo.connection.prepare("SELECT id,action,metadata,created_at FROM audit_log WHERE action IN ('agent_annotation','agent_assessment','agent_crop','agent_restore','review_text') ORDER BY id"))?;
    let rows = storage(query.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    }))?;
    for row in rows {
        let (event_id, action, metadata, time) = storage(row)?;
        let (media_id, actor, data) = if action != "review_text" {
            let data: Value = storage(serde_json::from_str(&metadata))?;
            (
                data["id"].as_i64().or_else(|| data["media_id"].as_i64()),
                "agent",
                data,
            )
        } else {
            let id = metadata
                .split_whitespace()
                .next()
                .and_then(|v| v.strip_prefix("media="))
                .and_then(|v| v.parse::<i64>().ok());
            (id, "human", Value::String(metadata))
        };
        if let Some(id) = media_id.filter(|id| job.accepted.contains(id)) {
            histories
                .entry(id)
                .or_default()
                .push(json!({"event_id":event_id,"action":action,"recorded_at":time,"actor":actor,"data":data}));
        }
    }
    for id in &job.accepted {
        let Some(media) = repo.media_by_id(*id)? else {
            continue;
        };
        let roots = if job.export_layout == "multi-resolution" {
            bdm_domain::training::RESOLUTIONS
                .into_iter()
                .map(|r| root.join(r.to_string()))
                .collect::<Vec<_>>()
        } else if job.export_layout == "flat" {
            vec![root.to_owned()]
        } else {
            vec![root.join(format!("media_{id}"))]
        };
        let history = histories.get(id).cloned().unwrap_or_default();
        let texts=artifacts.iter().filter(|a|a.media_id==*id).map(|a|json!({"kind":a.kind,"sha256":format!("{:x}",Sha256::digest(a.content.as_bytes())),"reviewed":a.reviewed,"provenance":a.provenance})).collect::<Vec<_>>();
        for directory in roots {
            let manifest = if matches!(job.export_layout.as_str(), "flat" | "multi-resolution") {
                directory.join(format!(".bdm-manifests/media_{id}.json"))
            } else {
                directory.join("manifest.json")
            };
            if !manifest.is_file() {
                continue;
            }
            let manifest: Value = storage(serde_json::from_slice(&storage(fs::read(&manifest))?))?;
            let name = manifest["image"].as_str().ok_or(IngestError::Integrity)?;
            if manifest["media_id"] != *id || !super::valid_image_name(&format!("media_{id}"), name)
            {
                return Err(IngestError::Integrity);
            }
            let image = directory.join(name);
            if !image.is_file() {
                continue;
            }
            let relative = storage(image.strip_prefix(root))?
                .to_string_lossy()
                .replace('\\', "/");
            let detail = details.get(id);
            let record = json!({"schema":"bdm-annotations-v1","media_id":id,"image":relative,"image_sha256":manifest["sha256"],
                "coordinate_space":"source_current_image","coordinate_sha256":detail.and_then(|d|d.image_sha256.as_deref()),
                "annotation_matches_source_current":detail.and_then(|d|d.image_sha256.as_deref()).map(|sha|sha==media.sha256),
                "source_current_sha256":media.sha256,"source_current_dimensions":media.dimensions(),"export_transform":manifest.get("training"),
                "source":media.post,"annotation_details":detail,"text_artifacts":texts,"exported_text_kind":manifest["text_kind"],
                "exported_text_sha256":manifest["text_sha256"],"history":history});
            // A JPEG->PNG refresh keeps one current index row in each export
            // directory; separate multiresolution directories remain distinct.
            entries.retain(|key, value| {
                value["media_id"] != *id || Path::new(key).parent() != Path::new(&relative).parent()
            });
            entries.insert(relative, record);
        }
    }
    let mut bytes = Vec::new();
    entries.retain(|key, _| root.join(key).is_file());
    for entry in entries.values() {
        storage(serde_json::to_writer(&mut bytes, entry))?;
        bytes.push(b'\n');
    }
    storage(fs::create_dir_all(root.join(".bdm-manifests")))?;
    // The receipt carries a pending digest so a restart can finish a previous
    // index write without accepting arbitrary user-modified index bytes.
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let pending = json!({"schema":"bdm-annotation-index-receipt-v1","sha256":if path.is_file(){Some(file_sha(&path)?)}else{None},"pending_sha256":digest});
    atomic_write(&receipt, &storage(serde_json::to_vec(&pending))?)?;
    let marker = json!({"schema":"bdm-annotation-index-receipt-v1","sha256":digest});
    atomic_write(&path, &bytes)?;
    atomic_write(&receipt, &storage(serde_json::to_vec(&marker))?)?;
    Ok(())
}
