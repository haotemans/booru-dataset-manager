use crate::{catalog::SqliteCatalog, ingest::storage};
use bdm_ports::IngestError;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};

pub struct AgentSnapshot {
    pub revision: i64,
    pub rejected: BTreeSet<usize>,
}

pub fn snapshot(path: &Path) -> Result<AgentSnapshot, IngestError> {
    let connection = storage(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    ))?;
    storage(connection.busy_timeout(Duration::from_millis(100)))?;
    let revision = storage(connection.query_row(
        "SELECT coalesce(max(id),0) FROM audit_log WHERE action LIKE 'agent_%'",
        [],
        |r| r.get(0),
    ))?;
    let mut query = storage(connection.prepare("SELECT a.media_id FROM agent_reviews a JOIN media m ON a.media_id=m.id WHERE a.decision='reject' AND a.image_sha256=json_extract(m.metadata,'$.sha256')"))?;
    let ids: Vec<i64> = storage(
        query
            .query_map([], |r| r.get(0))
            .and_then(|rows| rows.collect()),
    )?;
    Ok(AgentSnapshot {
        revision,
        rejected: ids
            .into_iter()
            .filter_map(|id| usize::try_from(id).ok())
            .collect(),
    })
}

impl SqliteCatalog {
    pub fn clear_agent_rejections(&mut self, ids: &BTreeSet<i64>) -> Result<(), IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        let tx = storage(self.connection.transaction())?;
        for id in ids {
            storage(tx.execute("UPDATE agent_reviews SET decision='defer',survivor_id=NULL,survivor_sha256=NULL WHERE media_id=?1",[id]))?;
        }
        storage(tx.execute(
            "INSERT INTO audit_log(action,metadata) VALUES('agent_review_cancel',?1)",
            [storage(serde_json::to_string(ids))?],
        ))?;
        storage(tx.commit())
    }

    /// Final UI confirmation, including AI duplicate association transfer.
    pub fn delete_review_scope(&mut self, ids: &BTreeSet<i64>) -> Result<(), IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        let mut groups: BTreeMap<Option<i64>, BTreeSet<i64>> = BTreeMap::new();
        for id in ids {
            let row:Option<(String,Option<i64>,Option<String>)>=storage(self.connection.query_row("SELECT image_sha256,survivor_id,survivor_sha256 FROM agent_reviews WHERE media_id=?1 AND decision='reject'",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional())?;
            let survivor = if let Some((sha, survivor, survivor_sha)) = row {
                if self.media_by_id(*id)?.is_none_or(|m| m.sha256 != sha) {
                    return Err(IngestError::Integrity);
                }
                if let Some(keep) = survivor {
                    if ids.contains(&keep)
                        || self
                            .media_by_id(keep)?
                            .is_none_or(|m| Some(m.sha256) != survivor_sha)
                    {
                        return Err(IngestError::Integrity);
                    }
                }
                survivor
            } else {
                None
            };
            groups.entry(survivor).or_default().insert(*id);
        }
        for (survivor, group) in groups {
            self.delete_media_locked(
                &group,
                survivor,
                "confirmed review deletion; preserve agent duplicate associations",
            )?;
        }
        Ok(())
    }

    pub(crate) fn agent_review_value(
        &self,
        id: i64,
    ) -> Result<Option<serde_json::Value>, IngestError> {
        let row: Option<(String, String, String)> = storage(
            self.connection
                .query_row(
                    "SELECT image_sha256,decision,report FROM agent_reviews WHERE media_id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional(),
        )?;
        Ok(row.map(|(sha,decision,report)|serde_json::json!({"image_sha256":sha,"decision":decision,"report":serde_json::from_str::<serde_json::Value>(&report).unwrap_or_default()})))
    }
}

pub(super) fn save_review(
    repo: &mut SqliteCatalog,
    id: i64,
    sha: &str,
    decision: &str,
    report: &serde_json::Value,
    survivor: Option<(i64, &str)>,
) -> Result<(), IngestError> {
    storage(repo.connection.execute("INSERT INTO agent_reviews(media_id,image_sha256,decision,report,survivor_id,survivor_sha256) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(media_id) DO UPDATE SET image_sha256=excluded.image_sha256,decision=excluded.decision,report=excluded.report,survivor_id=excluded.survivor_id,survivor_sha256=excluded.survivor_sha256",params![id,sha,decision,storage(serde_json::to_string(report))?,survivor.map(|v|v.0),survivor.map(|v|v.1)]))?;
    Ok(())
}
