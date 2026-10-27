use crate::{catalog::SqliteCatalog, ingest::storage};
use bdm_domain::{
    dedup::DuplicateCandidate,
    ingest::StoredMedia,
    tags::{NormalizedTags, TagRules},
};
use bdm_ports::IngestError;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, io::Write, path::Path};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkflowJob {
    pub id: i64,
    pub kind: String,
    pub ids: Vec<i64>,
    pub cursor: usize,
    pub state: String,
    pub error: Option<String>,
    pub profile_id: String,
    pub mode: String,
    pub accepted: BTreeSet<i64>,
    pub deferred: Vec<i64>,
    pub fallback: usize,
    pub recovery: Vec<i64>,
    pub destination: String,
    pub analysis_attempts: std::collections::BTreeMap<i64, u32>,
    pub analysis_errors: std::collections::BTreeMap<i64, Vec<String>>,
    pub analysis_scope: BTreeSet<i64>,
    pub review_similarity: Option<f32>,
    pub dedup_removed: usize,
    pub dedup_stats_recorded: bool,
    pub dedup_review_pairs: BTreeSet<(i64, i64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextArtifact {
    pub media_id: i64,
    pub kind: String,
    pub content: String,
    pub state: String,
    pub reviewed: bool,
    pub provenance: String,
}

impl TextArtifact {
    /// Human confirmation is independent of whether a usable output exists.
    pub fn is_generated_output(&self) -> bool {
        self.provenance != "local-normalization-v1"
            && !self.provenance.is_empty()
            && matches!(self.state.as_str(), "complete" | "needs_review")
            && !self.content.trim().is_empty()
    }
}

/// DCT-based 64-bit pHash; distance is a candidate signal, never byte identity.
pub fn phash(path: &Path) -> Result<u64, IngestError> {
    let image = storage(image::open(path))?
        .resize_exact(32, 32, image::imageops::FilterType::Lanczos3)
        .to_luma8();
    // pHash evaluates 64 low-frequency DCT coefficients. Precompute the 256
    // cosine terms once per image instead of computing ~131k transcendental
    // functions in the innermost pixel loops. Accumulation order is unchanged.
    let cosine: [[f64; 32]; 8] = std::array::from_fn(|frequency| {
        std::array::from_fn(|position| {
            ((2 * position + 1) as f64 * frequency as f64 * std::f64::consts::PI / 64.0).cos()
        })
    });
    let mut values = Vec::with_capacity(64);
    for v in 0..8 {
        for u in 0..8 {
            let mut sum = 0.0;
            for y in 0..32 {
                for x in 0..32 {
                    sum += f64::from(image.get_pixel(x, y).0[0])
                        * cosine[u as usize][x as usize]
                        * cosine[v as usize][y as usize];
                }
            }
            values.push(sum);
        }
    }
    let mut sorted = values[1..].to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    Ok(values
        .iter()
        .enumerate()
        .skip(1)
        .fold(0, |hash, (i, v)| hash | u64::from(*v > median) << i))
}

impl SqliteCatalog {
    pub fn tag_rules(&self) -> Result<TagRules, IngestError> {
        let json: String = storage(self.connection.query_row(
            "SELECT metadata FROM tag_rules WHERE id=1",
            [],
            |row| row.get(0),
        ))?;
        storage(serde_json::from_str(&json))
    }
    pub fn save_tag_rules(&mut self, rules: &TagRules) -> Result<(), IngestError> {
        rules
            .validate_characters()
            .map_err(|_| IngestError::InvalidMedia)?;
        let json = storage(serde_json::to_string(rules))?;
        let tx = storage(
            self.connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
        )?;
        storage(tx.execute(
            "INSERT INTO audit_log(action,metadata) VALUES('tag_rules',?1)",
            [&json],
        ))?;
        storage(tx.execute("UPDATE tag_rules SET metadata=?1 WHERE id=1", [&json]))?;
        storage(tx.commit())
    }
    pub fn recover_workflows(&mut self) -> Result<(), IngestError> {
        for mut job in self.list_workflows()? {
            if matches!(job.state.as_str(), "running" | "queued") {
                job.state = "interrupted".into();
                self.save_workflow(&job)?;
            }
        }
        Ok(())
    }
    pub fn list_artifacts(&self) -> Result<Vec<TextArtifact>, IngestError> {
        let mut query = storage(self.connection.prepare(
            "SELECT media_id,kind,content,state,reviewed,provenance FROM text_artifacts WHERE is_current=1",
        ))?;
        storage(
            query
                .query_map([], |row| {
                    Ok(TextArtifact {
                        media_id: row.get(0)?,
                        kind: row.get(1)?,
                        content: row.get(2)?,
                        state: row.get(3)?,
                        reviewed: row.get(4)?,
                        provenance: row.get(5)?,
                    })
                })
                .and_then(|rows| rows.collect()),
        )
    }
    pub fn review_artifact(&mut self, id: i64, kind: &str) -> Result<(), IngestError> {
        storage(self.connection.execute("UPDATE text_artifacts SET reviewed=1,state='complete' WHERE media_id=?1 AND kind=?2 AND is_current=1",params![id,kind]))?;
        self.audit("review_text", &format!("media={id} kind={kind}"))
    }
    pub fn write_artifact(
        &mut self,
        media: &StoredMedia,
        kind: &str,
        content: &str,
        provenance: &str,
        state: &str,
    ) -> Result<(), IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        self.write_artifact_locked(media, kind, content, provenance, state, false)
    }

    /// An explicit new AI optimization may replace a previously reviewed version.
    /// Source provenance is untouched; the old artifact remains in version history.
    pub fn write_ai_artifact(
        &mut self,
        media: &StoredMedia,
        kind: &str,
        content: &str,
        provenance: &str,
    ) -> Result<(), IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        self.write_artifact_locked(media, kind, content, provenance, "complete", true)
    }

    fn write_artifact_locked(
        &mut self,
        media: &StoredMedia,
        kind: &str,
        content: &str,
        provenance: &str,
        state: &str,
        replace_reviewed: bool,
    ) -> Result<(), IngestError> {
        if !matches!(kind, "tags" | "caption") || content.len() > 64 * 1024 {
            return Err(IngestError::InvalidMedia);
        }
        let tx = storage(
            self.connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
        )?;
        let reviewed:Option<bool>=storage(tx.query_row("SELECT reviewed FROM text_artifacts WHERE media_id=?1 AND kind=?2 AND is_current=1",params![media.id,kind],|row|row.get(0)).optional())?;
        if reviewed == Some(true) && !replace_reviewed {
            return Ok(());
        }
        let target = media
            .path
            .with_extension(if kind == "tags" { "tags.txt" } else { "txt" });
        let mut temporary = storage(tempfile::NamedTempFile::new_in(
            target.parent().ok_or(IngestError::Storage)?,
        ))?;
        storage(temporary.write_all(content.as_bytes()))?;
        storage(temporary.as_file().sync_all())?;
        storage(tx.execute(
            "UPDATE text_artifacts SET is_current=0 WHERE media_id=?1 AND kind=?2 AND is_current=1",
            params![media.id, kind],
        ))?;
        storage(tx.execute("INSERT INTO text_artifacts(media_id,kind,content,sha256,provenance,state) VALUES(?1,?2,?3,?4,?5,?6)",params![media.id,kind,content,format!("{:x}",Sha256::digest(content.as_bytes())),provenance,state]))?;
        storage(tx.commit())?;
        // Database content is authoritative; startup materializes interrupted sidecar writes.
        storage(temporary.persist(&target))?;
        Ok(())
    }
    pub fn materialize_artifacts(&self) -> Result<(), IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        let media = self.list_media()?;
        for artifact in self.list_artifacts()? {
            if let Some(item) = media.iter().find(|m| m.id == artifact.media_id) {
                let path = item.path.with_extension(if artifact.kind == "tags" {
                    "tags.txt"
                } else {
                    "txt"
                });
                if fs::read_to_string(&path).ok().as_deref() != Some(&artifact.content) {
                    let mut temp = storage(tempfile::NamedTempFile::new_in(
                        path.parent().ok_or(IngestError::Storage)?,
                    ))?;
                    storage(temp.write_all(artifact.content.as_bytes()))?;
                    storage(temp.as_file().sync_all())?;
                    storage(temp.persist(path))?;
                }
            }
        }
        Ok(())
    }
    pub fn normalize_media(&mut self, media: &StoredMedia) -> Result<NormalizedTags, IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        self.normalize_media_locked(media)
    }

    pub(crate) fn normalize_media_locked(
        &mut self,
        media: &StoredMedia,
    ) -> Result<NormalizedTags, IngestError> {
        let rules = self.tag_rules()?;
        let result = bdm_domain::tags::normalize(
            &media.post.tag_string,
            &media.post.tag_string_artist,
            &media.post.tag_string_character,
            &rules,
        );
        let current: Option<String> = storage(self.connection.query_row(
            "SELECT provenance FROM text_artifacts WHERE media_id=?1 AND kind='tags' AND is_current=1",
            [media.id], |r| r.get(0),
        ).optional())?;
        // Metadata refresh cannot replace an AI or manually edited artifact.
        if current.is_some_and(|p| p != "local-normalization-v1") {
            if !result.warnings.is_empty() {
                storage(self.connection.execute("UPDATE text_artifacts SET state='needs_review' WHERE media_id=?1 AND kind='tags' AND is_current=1 AND reviewed=0",[media.id]))?;
            }
            return Ok(result);
        }
        self.write_artifact_locked(
            media,
            "tags",
            &result.tags.join(", "),
            "local-normalization-v1",
            if result.warnings.is_empty() {
                "complete"
            } else {
                "needs_review"
            },
            false,
        )?;
        Ok(result)
    }
    pub fn new_workflow(&mut self, kind: &str, ids: Vec<i64>) -> Result<WorkflowJob, IngestError> {
        let tx = storage(
            self.connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
        )?;
        storage(tx.execute(
            "INSERT INTO workflow_jobs(kind,metadata) VALUES(?1,'{}')",
            [kind],
        ))?;
        let job = WorkflowJob {
            id: tx.last_insert_rowid(),
            kind: kind.into(),
            ids,
            state: "queued".into(),
            fallback: 8,
            ..Default::default()
        };
        storage(tx.execute(
            "UPDATE workflow_jobs SET metadata=?2 WHERE id=?1",
            params![job.id, storage(serde_json::to_string(&job))?],
        ))?;
        storage(tx.commit())?;
        Ok(job)
    }
    pub fn save_workflow(&mut self, job: &WorkflowJob) -> Result<(), IngestError> {
        storage(self.connection.execute(
            "UPDATE workflow_jobs SET metadata=?2 WHERE id=?1",
            params![job.id, storage(serde_json::to_string(job))?],
        ))?;
        Ok(())
    }
    pub fn list_workflows(&self) -> Result<Vec<WorkflowJob>, IngestError> {
        let mut query = storage(
            self.connection
                .prepare("SELECT metadata FROM workflow_jobs ORDER BY id DESC"),
        )?;
        storage(
            query
                .query_map([], |row| row.get::<_, String>(0))
                .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>()),
        )?
        .into_iter()
        .map(|s| storage(serde_json::from_str(&s)))
        .collect()
    }
    pub fn list_candidates(&self) -> Result<Vec<DuplicateCandidate>, IngestError> {
        let mut query = storage(
            self.connection
                .prepare("SELECT metadata FROM duplicate_candidates ORDER BY first_id,second_id"),
        )?;
        storage(
            query
                .query_map([], |row| row.get::<_, String>(0))
                .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>()),
        )?
        .into_iter()
        .map(|s| storage(serde_json::from_str(&s)))
        .collect()
    }
    pub fn save_candidate(&mut self, pair: &DuplicateCandidate) -> Result<(), IngestError> {
        if self.candidate_rejected(pair.first, pair.second)? {
            return Ok(());
        }
        storage(self.connection.execute(
            "INSERT OR REPLACE INTO duplicate_candidates VALUES(?1,?2,?3)",
            params![
                pair.first,
                pair.second,
                storage(serde_json::to_string(pair))?
            ],
        ))?;
        Ok(())
    }
    pub fn candidate_rejected(&self, first: i64, second: i64) -> Result<bool, IngestError> {
        let json: Option<String> = storage(
            self.connection
                .query_row(
                    "SELECT metadata FROM duplicate_candidates WHERE first_id=?1 AND second_id=?2",
                    params![first, second],
                    |r| r.get(0),
                )
                .optional(),
        )?;
        json.map(|json| {
            storage(serde_json::from_str::<DuplicateCandidate>(&json))
                .map(|p| p.state == "rejected")
        })
        .transpose()
        .map(|v| v.unwrap_or(false))
    }
    pub fn audit(&self, action: &str, metadata: &str) -> Result<(), IngestError> {
        storage(self.connection.execute(
            "INSERT INTO audit_log(action,metadata) VALUES(?1,?2)",
            params![action, metadata],
        ))?;
        Ok(())
    }
    pub fn delete_media(
        &mut self,
        ids: &BTreeSet<i64>,
        survivor: Option<i64>,
        reason: &str,
    ) -> Result<(), IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        self.delete_media_locked(ids, survivor, reason)
    }

    /// Persist the exact pair reviewed by the user; stale/replaced images require a new review.
    pub fn resolve_candidate(
        &mut self,
        pair: &DuplicateCandidate,
        hashes: &[String; 2],
        survivor: Option<i64>,
    ) -> Result<(), IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        if pair.state != "pending"
            || pair.first >= pair.second
            || survivor.is_some_and(|id| id != pair.first && id != pair.second)
        {
            return Err(IngestError::Integrity);
        }
        let saved: Option<String> = storage(
            self.connection
                .query_row(
                    "SELECT metadata FROM duplicate_candidates WHERE first_id=?1 AND second_id=?2",
                    params![pair.first, pair.second],
                    |row| row.get(0),
                )
                .optional(),
        )?;
        let saved = saved
            .map(|json| storage(serde_json::from_str::<DuplicateCandidate>(&json)))
            .transpose()?;
        if saved.as_ref() != Some(pair) {
            return Err(IngestError::Integrity);
        }
        for (id, hash) in [pair.first, pair.second].into_iter().zip(hashes) {
            if self
                .media_by_id(id)?
                .is_none_or(|media| media.sha256 != *hash)
            {
                return Err(IngestError::Integrity);
            }
        }
        if let Some(keep) = survivor {
            let remove = if keep == pair.first {
                pair.second
            } else {
                pair.first
            };
            self.delete_media_locked(
                &BTreeSet::from([remove]),
                Some(keep),
                "human-confirmed visual comparison; preserve associations",
            )
        } else {
            let mut rejected = pair.clone();
            rejected.state = "rejected".into();
            let tx = storage(
                self.connection
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
            )?;
            storage(tx.execute(
                "UPDATE duplicate_candidates SET metadata=?3 WHERE first_id=?1 AND second_id=?2",
                params![
                    pair.first,
                    pair.second,
                    storage(serde_json::to_string(&rejected))?
                ],
            ))?;
            storage(tx.execute(
                "INSERT INTO audit_log(action,metadata) VALUES('review_similarity_keep_both',?1)",
                [format!("first={} second={}", pair.first, pair.second)],
            ))?;
            storage(tx.commit())
        }
    }

    /// Explicitly undo an ignored pair without re-analysis or modifying either original.
    pub fn restore_candidate(
        &mut self,
        pair: &DuplicateCandidate,
        hashes: &[String; 2],
    ) -> Result<(), IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        if pair.state != "rejected" || pair.first >= pair.second {
            return Err(IngestError::Integrity);
        }
        let tx = storage(
            self.connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
        )?;
        let saved: Option<String> = storage(
            tx.query_row(
                "SELECT metadata FROM duplicate_candidates WHERE first_id=?1 AND second_id=?2",
                params![pair.first, pair.second],
                |row| row.get(0),
            )
            .optional(),
        )?;
        if saved
            .map(|json| storage(serde_json::from_str::<DuplicateCandidate>(&json)))
            .transpose()?
            .as_ref()
            != Some(pair)
        {
            return Err(IngestError::Integrity);
        }
        for (id, hash) in [pair.first, pair.second].into_iter().zip(hashes) {
            let current: Option<String> = storage(
                tx.query_row("SELECT sha256 FROM media WHERE id=?1", [id], |row| {
                    row.get(0)
                })
                .optional(),
            )?;
            if current.as_ref() != Some(hash) {
                return Err(IngestError::Integrity);
            }
        }
        let mut pending = pair.clone();
        pending.state = "pending".into();
        storage(tx.execute(
            "UPDATE duplicate_candidates SET metadata=?3 WHERE first_id=?1 AND second_id=?2",
            params![
                pair.first,
                pair.second,
                storage(serde_json::to_string(&pending))?
            ],
        ))?;
        storage(tx.execute(
            "INSERT INTO audit_log(action,metadata) VALUES('review_similarity_restore',?1)",
            [format!("first={} second={}", pair.first, pair.second)],
        ))?;
        storage(tx.commit())
    }

    fn delete_media_locked(
        &mut self,
        ids: &BTreeSet<i64>,
        survivor: Option<i64>,
        reason: &str,
    ) -> Result<(), IngestError> {
        let all = ids
            .iter()
            .map(|id| self.media_by_id(*id)?.ok_or(IngestError::Storage))
            .collect::<Result<Vec<_>, _>>()?;
        let retained = survivor
            .map(|id| self.media_by_id(id)?.ok_or(IngestError::Storage))
            .transpose()?;
        if ids.is_empty() || survivor.is_some_and(|id| ids.contains(&id)) {
            return Err(IngestError::Storage);
        }
        let tx = storage(
            self.connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
        )?;
        for item in all.iter().filter(|m| ids.contains(&m.id)) {
            if let Some(keep) = survivor {
                storage(tx.execute("INSERT OR IGNORE INTO collection_media SELECT collection_id,?2 FROM collection_media WHERE media_id=?1",params![item.id,keep]))?;
                storage(tx.execute(
                    "UPDATE media_sources SET media_id=?2 WHERE media_id=?1",
                    params![item.id, keep],
                ))?;
                storage(tx.execute(
                    "UPDATE text_artifacts SET is_current=0 WHERE media_id=?1",
                    [item.id],
                ))?;
                storage(tx.execute(
                    "UPDATE text_artifacts SET media_id=?2 WHERE media_id=?1",
                    params![item.id, keep],
                ))?;
            }
            storage(tx.execute(
                "INSERT OR REPLACE INTO deletion_journal VALUES(?1,?2)",
                params![item.id, storage(serde_json::to_string(item))?],
            ))?;
            storage(tx.execute(
                "INSERT INTO audit_log(action,metadata) VALUES('delete',?1)",
                [format!(
                        "removed={} retained={} reason={reason}",
                        item.path.display(),
                        retained.as_ref()
                            .map(|m| m.path.display().to_string())
                            .unwrap_or_else(|| "none".into())
                    )],
            ))?;
        }
        storage(tx.commit())?;
        self.recover_deletions_locked()
    }
    pub fn recover_deletions(&mut self) -> Result<(), IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        self.recover_deletions_locked()
    }
    fn recover_deletions_locked(&mut self) -> Result<(), IngestError> {
        let pending: Vec<String> = {
            let mut q = storage(
                self.connection
                    .prepare("SELECT metadata FROM deletion_journal"),
            )?;
            storage(
                q.query_map([], |r| r.get(0))
                    .and_then(|rows| rows.collect()),
            )?
        };
        for json in pending {
            let media: StoredMedia = storage(serde_json::from_str(&json))?;
            let directory = media.path.parent().ok_or(IngestError::Storage)?;
            if directory.file_name().and_then(|v| v.to_str()) != Some(&media.post.md5)
                || directory
                    .parent()
                    .and_then(|v| v.file_name())
                    .and_then(|v| v.to_str())
                    != Some("originals")
            {
                return Err(IngestError::Storage);
            }
            if directory.exists() {
                let resolved = storage(directory.canonicalize())?;
                let parent = storage(
                    directory
                        .parent()
                        .ok_or(IngestError::Storage)?
                        .canonicalize(),
                )?;
                if resolved.parent() != Some(parent.as_path())
                    || storage(fs::symlink_metadata(directory))?
                        .file_type()
                        .is_symlink()
                {
                    return Err(IngestError::Storage);
                }
                storage(fs::remove_dir_all(directory))?;
            }
            let tx = storage(
                self.connection
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
            )?;
            storage(tx.execute("DELETE FROM media WHERE id=?1", [media.id]))?;
            storage(tx.execute("DELETE FROM deletion_journal WHERE media_id=?1", [media.id]))?;
            storage(tx.commit())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod phash_performance_regression {
    use super::*;

    #[test]
    fn cached_cosines_preserve_original_phash_bits() {
        let dir = tempfile::tempdir().unwrap();
        for seed in 0..3u32 {
            let path = dir.path().join(format!("test-{seed}.png"));
            let image = image::RgbImage::from_fn(96, 72, |x, y| {
                image::Rgb([
                    ((x * 7 + y * 3 + seed * 13) % 255) as u8,
                    ((x * y + seed * 17) % 255) as u8,
                    ((x * 9 + y + seed * 41) % 255) as u8,
                ])
            });
            image.save(&path).unwrap();
            let resized = image::open(&path)
                .unwrap()
                .resize_exact(32, 32, image::imageops::FilterType::Lanczos3)
                .to_luma8();
            let mut values = Vec::with_capacity(64);
            for v in 0..8 {
                for u in 0..8 {
                    let mut sum = 0.0;
                    for y in 0..32 {
                        for x in 0..32 {
                            sum += f64::from(resized.get_pixel(x, y).0[0])
                                * ((f64::from(2 * x + 1) * u as f64 * std::f64::consts::PI) / 64.0)
                                    .cos()
                                * ((f64::from(2 * y + 1) * v as f64 * std::f64::consts::PI) / 64.0)
                                    .cos();
                        }
                    }
                    values.push(sum);
                }
            }
            let mut sorted = values[1..].to_vec();
            sorted.sort_by(f64::total_cmp);
            let median = sorted[sorted.len() / 2];
            let expected = values
                .iter()
                .enumerate()
                .skip(1)
                .fold(0u64, |h, (i, v)| h | u64::from(*v > median) << i);
            assert_eq!(phash(&path).unwrap(), expected);
        }
    }
}
