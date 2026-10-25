use crate::{
    catalog::SqliteCatalog,
    ingest::storage,
    workflows::{phash, WorkflowJob},
};
use bdm_domain::dedup::{prefer_first, DuplicateCandidate, SurvivorQuality};
use bdm_ports::IngestError;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

const WORKER: &str = include_str!("../../../scripts/dinov3_worker.py");
pub const VERSION:&str="timm/vit_small_patch16_dinov3.lvd1689m@3bf4720a82ec2066db88137180ff1f83a675cef0:rgb224-bicubic-imagenet-cls-l2-v1";

fn valid_vector(vector: &[f32]) -> bool {
    vector.len() == 384
        && vector.iter().all(|v| v.is_finite())
        && (vector.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() <= 0.001
}

fn buckets(hash: u64, vector: Option<&[f32]>) -> Vec<String> {
    let mut result = vec![
        format!("p0:{}", hash & 0x1fffff),
        format!("p1:{}", (hash >> 21) & 0x1fffff),
        format!("p2:{}", hash >> 42),
    ];
    if let Some(vector) = vector {
        for table in 0..6u64 {
            let mut signature = 0u16;
            for bit in 0..12u64 {
                let score: f32 = vector
                    .iter()
                    .enumerate()
                    .map(|(dimension, value)| {
                        let mut seed = (dimension as u64 + 1).wrapping_mul(0x9e3779b97f4a7c15)
                            ^ (table * 12 + bit + 1).wrapping_mul(0xbf58476d1ce4e5b9);
                        seed ^= seed >> 30;
                        seed = seed.wrapping_mul(0xbf58476d1ce4e5b9);
                        seed ^= seed >> 27;
                        if seed & 1 == 0 {
                            *value
                        } else {
                            -*value
                        }
                    })
                    .sum();
                if score >= 0.0 {
                    signature |= 1 << bit;
                }
            }
            result.push(format!("v{table}:{signature}"));
        }
    }
    result
}

fn candidate_features(
    repo: &SqliteCatalog,
    id: i64,
    keys: &[String],
    review: Option<(f32, &[f32])>,
) -> Result<Vec<(i64, String, Option<String>)>, IngestError> {
    if let Some((threshold, vector)) = review {
        let count: i64 = storage(repo.connection.query_row(
            "SELECT count(*) FROM visual_features WHERE version=?1",
            [VERSION],
            |row| row.get(0),
        ))?;
        if count <= 2000 {
            // Exact streamed retrieval for small libraries avoids hash-bucket misses on background variants.
            let mut query = storage(repo.connection.prepare("SELECT media_id,phash,embedding FROM visual_features WHERE version=?1 AND media_id<>?2 ORDER BY media_id"))?;
            let rows = storage(query.query_map(params![VERSION, id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            }))?;
            let mut matches = Vec::new();
            for row in rows {
                let row = storage(row)?;
                let other_hash =
                    u64::from_str_radix(&row.1, 16).map_err(|_| IngestError::Storage)?;
                let other_keys = buckets(other_hash, None);
                let hash_match = keys[..3].iter().any(|key| other_keys.contains(key));
                let score = row
                    .2
                    .as_ref()
                    .and_then(|json| serde_json::from_str::<Vec<f32>>(json).ok())
                    .filter(|v| valid_vector(v))
                    .map(|v| vector.iter().zip(v).map(|(a, b)| a * b).sum::<f32>());
                if hash_match || score.is_some_and(|score| score >= threshold) {
                    matches.push(row);
                }
            }
            return Ok(matches);
        }
    }
    let mut ids = BTreeSet::new();
    let mut search_keys = keys.to_vec();
    if review.is_some() {
        for key in &keys[3..] {
            if let Some((table, signature)) = key.split_once(':') {
                if let Ok(signature) = signature.parse::<u16>() {
                    for bit in 0..12 {
                        search_keys.push(format!("{table}:{}", signature ^ (1 << bit)));
                    }
                }
            }
        }
    }
    for key in &search_keys {
        let mut query=storage(repo.connection.prepare("SELECT media_id FROM visual_buckets WHERE version=?1 AND bucket=?2 AND media_id<>?3 ORDER BY media_id DESC LIMIT 128"))?;
        let rows = storage(query.query_map(params![VERSION, key, id], |r| r.get::<_, i64>(0)))?;
        for other in rows {
            ids.insert(storage(other)?);
        }
    }
    ids.into_iter().take(512).map(|other|storage(repo.connection.query_row("SELECT media_id,phash,embedding FROM visual_features WHERE version=?1 AND media_id=?2",params![VERSION,other],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))))).collect()
}

fn analysis_python(executable: Option<&Path>, override_path: Option<PathBuf>) -> PathBuf {
    if let Some(path) = override_path {
        return path;
    }
    let runtime = if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    };
    if let Some(parent) = executable.and_then(Path::parent) {
        for directory in parent.ancestors() {
            for relative in ["analysis-runtime", "target/analysis-runtime"] {
                let candidate = directory.join(relative).join(runtime);
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    "python".into()
}

pub fn python_command() -> Command {
    let executable = std::env::current_exe().ok();
    let python = analysis_python(
        executable.as_deref(),
        std::env::var_os("BDM_ANALYSIS_PYTHON").map(PathBuf::from),
    );
    #[cfg(windows)]
    let command = {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new(python);
        command.creation_flags(0x08000000);
        command
    };
    #[cfg(not(windows))]
    let command = Command::new(python);
    command
}

pub fn download_model(root: &Path) -> Result<(), IngestError> {
    let output = storage(
        python_command()
            .args(["-c", WORKER, "--model-dir"])
            .arg(root)
            .arg("--download")
            .stderr(Stdio::null())
            .output(),
    )?;
    if !output.status.success() {
        return Err(IngestError::Storage);
    }
    Ok(())
}

#[derive(Deserialize)]
struct Embedding {
    id: i64,
    version: String,
    vector: Vec<f32>,
}

struct Worker {
    child: Child,
    input: ChildStdin,
    output: Receiver<String>,
}
impl Worker {
    fn spawn(root: &Path) -> Result<Self, IngestError> {
        let mut child = python_command()
            .args(["-u", "-c", WORKER, "--model-dir"])
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| IngestError::Analysis)?;
        let input = child.stdin.take().ok_or(IngestError::Analysis)?;
        let output = child.stdout.take().ok_or(IngestError::Analysis)?;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                match line {
                    Ok(line) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            child,
            input,
            output: receiver,
        })
    }
    fn infer(
        &mut self,
        id: i64,
        path: &Path,
        cancel: &AtomicBool,
    ) -> Result<Vec<f32>, IngestError> {
        writeln!(self.input, "{}", serde_json::json!({"id":id,"path":path}))
            .map_err(|_| IngestError::Analysis)?;
        self.input.flush().map_err(|_| IngestError::Analysis)?;
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(IngestError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(IngestError::Analysis);
            }
            match self.output.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => {
                    let response: Embedding =
                        serde_json::from_str(&line).map_err(|_| IngestError::Analysis)?;
                    return if response.id == id
                        && response.version == VERSION
                        && valid_vector(&response.vector)
                    {
                        Ok(response.vector)
                    } else {
                        Err(IngestError::Analysis)
                    };
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(IngestError::Analysis),
            }
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn run(
    repo: &mut SqliteCatalog,
    job: &mut WorkflowJob,
    root: &Path,
    cancel: &AtomicBool,
    mut report: impl FnMut(&WorkflowJob),
) -> Result<(), IngestError> {
    let scope: BTreeSet<_> = if job.analysis_scope.is_empty() {
        job.ids.iter().copied().collect()
    } else {
        job.analysis_scope.clone()
    };
    let model_present = root.join("model.safetensors").is_file();
    let mut worker: Option<Worker> = None;
    job.state = "running".into();
    job.error = None;
    job.dedup_stats_recorded = true;
    let mut incomplete = false;
    repo.save_workflow(job)?;
    let result: Result<(), IngestError> = (|| {
        while job.cursor < job.ids.len() {
            if cancel.load(Ordering::Relaxed) {
                job.state = "paused".into();
                break;
            }
            let id = job.ids[job.cursor];
            let Some(media) = repo.media_by_id(id)? else {
                job.cursor += 1;
                continue;
            };
            let hash = match phash(&media.path) {
                Ok(hash) => hash,
                Err(error) => {
                    *job.analysis_attempts.entry(id).or_default() += 1;
                    job.analysis_errors
                        .entry(id)
                        .or_default()
                        .push(error.to_string());
                    if job.analysis_attempts[&id] < 3 {
                        repo.save_workflow(job)?;
                        continue;
                    }
                    if !job.deferred.contains(&id) {
                        job.deferred.push(id);
                    }
                    storage(repo.connection.execute(
                        "UPDATE analysis_jobs SET state='failed',reason=?2 WHERE media_id=?1",
                        params![id, error.to_string()],
                    ))?;
                    job.cursor += 1;
                    repo.save_workflow(job)?;
                    report(job);
                    continue;
                }
            };
            let existing: Option<String> = storage(
                repo.connection
                    .query_row(
                        "SELECT embedding FROM visual_features WHERE media_id=?1 AND version=?2",
                        params![id, VERSION],
                        |r| r.get(0),
                    )
                    .optional(),
            )?
            .flatten();
            let mut vector = existing
                .map(|json| storage(serde_json::from_str::<Vec<f32>>(&json)))
                .transpose()?;
            vector = vector.filter(|v| valid_vector(v));
            while vector.is_none()
                && model_present
                && *job.analysis_attempts.get(&id).unwrap_or(&0) < 3
            {
                if cancel.load(Ordering::Relaxed) {
                    job.state = "paused".into();
                    break;
                }
                *job.analysis_attempts.entry(id).or_default() += 1;
                repo.save_workflow(job)?;
                let inferred = (|| {
                    if worker.is_none() {
                        worker = Some(Worker::spawn(root)?);
                    }
                    worker
                        .as_mut()
                        .ok_or(IngestError::Analysis)?
                        .infer(id, &media.path, cancel)
                })();
                match inferred {
                    Ok(values) => vector = Some(values),
                    Err(IngestError::Cancelled) => {
                        job.state = "paused".into();
                        break;
                    }
                    Err(error) => {
                        worker = None;
                        job.analysis_errors
                            .entry(id)
                            .or_default()
                            .push(error.to_string());
                        repo.save_workflow(job)?;
                        if job.analysis_attempts[&id] < 3 {
                            let deadline = Instant::now()
                                + Duration::from_secs(u64::from(job.analysis_attempts[&id]));
                            while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
                                std::thread::sleep(Duration::from_millis(100));
                            }
                        }
                    }
                }
            }
            if job.state == "paused" {
                break;
            }
            if vector.is_none() && model_present && !job.deferred.contains(&id) {
                job.deferred.push(id);
            }
            incomplete |= vector.is_none();
            storage(repo.connection.execute(
                "INSERT OR REPLACE INTO visual_features VALUES(?1,?2,?3,?4)",
                params![
                        id,
                        VERSION,
                        format!("{hash:016x}"),
                        vector
                            .as_ref()
                            .map(serde_json::to_string)
                            .transpose()
                            .map_err(|_| IngestError::Storage)?
                    ],
            ))?;
            let keys = buckets(hash, vector.as_deref());
            for key in &keys {
                storage(repo.connection.execute(
                    "INSERT OR IGNORE INTO visual_buckets VALUES(?1,?2,?3)",
                    params![id, VERSION, key],
                ))?;
            }
            let threshold = job
                .review_similarity
                .filter(|value| value.is_finite())
                .map(|value| value.clamp(0.85, 0.995));
            let features = candidate_features(repo, id, &keys, threshold.zip(vector.as_deref()))?;
            for (other, other_hash, other_vector) in features {
                if job.kind != "dedup-auto" && !scope.contains(&other) {
                    continue;
                }
                let distance = (hash
                    ^ u64::from_str_radix(&other_hash, 16).map_err(|_| IngestError::Storage)?)
                .count_ones();
                let cosine = if let (Some(a), Some(b)) = (&vector, other_vector) {
                    let b: Vec<f32> = storage(serde_json::from_str(&b))?;
                    if !valid_vector(a) || !valid_vector(&b) {
                        None
                    } else {
                        Some(
                            a.iter()
                                .zip(b)
                                .map(|(x, y)| x * y)
                                .sum::<f32>()
                                .clamp(-1.0, 1.0),
                        )
                    }
                } else {
                    None
                };
                if distance > 2 && !cosine.is_some_and(|s| s >= threshold.unwrap_or(0.997)) {
                    continue;
                }
                let mut pair = DuplicateCandidate {
                    first: id.min(other),
                    second: id.max(other),
                    phash_distance: Some(distance),
                    cosine,
                    model_version: vector.as_ref().map(|_| VERSION.into()),
                    state: "pending".into(),
                };
                let rejected = repo.candidate_rejected(pair.first, pair.second)?;
                if rejected {
                    continue;
                }
                if pair.dual_confirmed() {
                    if let (Some(a), Some(b)) = (repo.media_by_id(id)?, repo.media_by_id(other)?) {
                        let quality = |m: &bdm_domain::ingest::StoredMedia| -> Result<SurvivorQuality,IngestError> {
                            let values:Option<(Option<f64>,Option<bool>)>=storage(repo.connection.query_row("SELECT aesthetic,watermark_free FROM media_quality WHERE media_id=?1",[m.id],|r|Ok((r.get(0)?,r.get(1)?))).optional())?;
                            let (aesthetic,watermark_free)=values.unwrap_or_default();
                            Ok(SurvivorQuality {pixels:u64::from(m.dimensions().0)*u64::from(m.dimensions().1),png:m.post.file_ext=="png",id:m.id,aesthetic,watermark_free})
                        };
                        let (keep, remove) = if prefer_first(quality(&a)?, quality(&b)?) {
                            (a.id, b.id)
                        } else {
                            (b.id, a.id)
                        };
                        let reason=format!("pHash={distance} cosine={cosine:?} model={VERSION}; priority aesthetic-if-both-known/pixels/PNG/confirmed-watermark-free/stable-id");
                        repo.delete_media(&BTreeSet::from([remove]), Some(keep), &reason)?;
                        job.dedup_removed += 1;
                        pair.state = "automatic_removed".into();
                        if remove == id {
                            break;
                        }
                    }
                } else {
                    repo.save_candidate(&pair)?;
                    job.dedup_review_pairs.insert((pair.first, pair.second));
                }
            }
            storage(repo.connection.execute(
                "UPDATE analysis_jobs SET state=?2,reason=?3 WHERE media_id=?1",
                params![
                    id,
                    if vector.is_some() {
                        "complete"
                    } else if model_present {
                        "failed"
                    } else {
                        "needs_model"
                    },
                    if vector.is_some() {
                        ""
                    } else if model_present {
                        "DINOv3 分析已失败三次，请检查模型与 Python 环境后手动重试"
                    } else {
                        "DINOv3 模型或本地运行环境尚未就绪；pHash 已完成"
                    }
                ],
            ))?;
            job.cursor += 1;
            repo.save_workflow(job)?;
            report(job);
        }
        if job.cursor == job.ids.len() {
            job.state = if !job.deferred.is_empty() {
                "deferred"
            } else if !incomplete {
                "completed"
            } else {
                "needs_model"
            }
            .into();
        }
        Ok(())
    })();
    drop(worker);
    if !job.deferred.is_empty() {
        job.error = Some(format!(
            "{} 张图片分析失败，成功特征已保留；继续任务将补试失败项。",
            job.deferred.len()
        ));
    }
    if let Err(error) = &result {
        job.state = "failed".into();
        job.error = Some(error.to_string());
    }
    repo.save_workflow(job)?;
    report(job);
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn packaged_python_resolution_uses_the_installed_location_and_honors_overrides() {
        let root = tempfile::tempdir().unwrap();
        let runtime = root
            .path()
            .join("target/analysis-runtime")
            .join(if cfg!(windows) {
                "Scripts/python.exe"
            } else {
                "bin/python"
            });
        std::fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        std::fs::write(&runtime, b"test fixture").unwrap();
        for relative in [
            "target/debug/bdm-desktop.exe",
            "target/actions-fixture/bdm-desktop.exe",
        ] {
            let exe = root.path().join(relative);
            assert_eq!(super::analysis_python(Some(&exe), None), runtime);
        }
        let explicit = root.path().join("custom/python.exe");
        assert_eq!(
            super::analysis_python(Some(&root.path().join("app.exe")), Some(explicit.clone())),
            explicit
        );
        let installed = root
            .path()
            .join("portable/analysis-runtime")
            .join(if cfg!(windows) {
                "Scripts/python.exe"
            } else {
                "bin/python"
            });
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::write(&installed, b"test fixture").unwrap();
        assert_eq!(
            super::analysis_python(Some(&root.path().join("portable/app.exe")), None),
            installed
        );
        assert_eq!(
            super::analysis_python(None, None),
            std::path::PathBuf::from("python")
        );
    }
    use super::*;
    use bdm_ports::IngestRepository;
    use sha2::Digest;

    #[test]
    fn relaxed_review_finds_cross_batch_variants_without_buckets_and_never_relaxes_deletion() {
        let mut f = crate::workflow_tests::fixture();
        let cancel = AtomicBool::new(false);
        let root = f.directory.path().join("library");
        f.repo
            .import(&f.post, &root, f.collection, 800, &cancel, &f.provider)
            .unwrap();
        let first = f.repo.list_media().unwrap().remove(0);
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(800, 800, |x, y| {
            image::Rgb([((x * 3 + y) % 255) as u8, ((y * 7) % 255) as u8, 220])
        }))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
        f.provider.bytes = bytes.into_inner();
        f.post.id = 101;
        f.post.md5 = format!("{:x}", md5::Md5::digest(&f.provider.bytes));
        f.post.file_size = f.provider.bytes.len() as u64;
        f.repo
            .import(&f.post, &root, f.collection, 800, &cancel, &f.provider)
            .unwrap();
        let second = f
            .repo
            .list_media()
            .unwrap()
            .into_iter()
            .find(|m| m.id != first.id)
            .unwrap();
        let a_hash = phash(&first.path).unwrap();
        let b_hash = phash(&second.path).unwrap();
        assert!((a_hash ^ b_hash).count_ones() > 2);
        for (media, cosine) in [(&first, 1.0_f32), (&second, 0.96_f32)] {
            let mut vector = vec![0.0; 384];
            vector[0] = cosine;
            vector[1] = (1.0 - cosine * cosine).sqrt();
            f.repo
                .connection
                .execute(
                    "INSERT INTO visual_features VALUES(?1,?2,?3,?4)",
                    params![
                        media.id,
                        VERSION,
                        format!("{:016x}", phash(&media.path).unwrap()),
                        serde_json::to_string(&vector).unwrap()
                    ],
                )
                .unwrap();
        }
        // Separate automatic batches still compare against the complete cached library.
        let mut strict = f.repo.new_workflow("dedup-auto", vec![second.id]).unwrap();
        run(
            &mut f.repo,
            &mut strict,
            f.directory.path(),
            &cancel,
            |_| {},
        )
        .unwrap();
        assert!(f.repo.list_candidates().unwrap().is_empty());
        f.repo
            .connection
            .execute("DELETE FROM visual_buckets", [])
            .unwrap();
        let mut relaxed = f.repo.new_workflow("dedup-auto", vec![second.id]).unwrap();
        relaxed.review_similarity = Some(0.95);
        run(
            &mut f.repo,
            &mut relaxed,
            f.directory.path(),
            &cancel,
            |_| {},
        )
        .unwrap();
        let pair = f.repo.list_candidates().unwrap().remove(0);
        assert_eq!((pair.first, pair.second), (first.id, second.id));
        assert!(!pair.dual_confirmed());
        assert_eq!(relaxed.dedup_removed, 0);
        assert!(relaxed.dedup_stats_recorded);
        assert_eq!(relaxed.dedup_review_pairs.len(), 1);
        assert_eq!(f.repo.list_media().unwrap().len(), 2);
        assert!(first.path.is_file() && second.path.is_file());
        // Repeated analysis counts each pair once and preserves human rejection after restart.
        let mut repeated = f
            .repo
            .new_workflow("dedup", vec![first.id, second.id])
            .unwrap();
        repeated.review_similarity = Some(0.95);
        run(
            &mut f.repo,
            &mut repeated,
            f.directory.path(),
            &cancel,
            |_| {},
        )
        .unwrap();
        assert_eq!(repeated.dedup_review_pairs.len(), 1);
        let mut rejected = pair;
        rejected.state = "rejected".into();
        f.repo.save_candidate(&rejected).unwrap();
        let mut reopened =
            SqliteCatalog::open(&f.directory.path().join("catalog.sqlite3")).unwrap();
        let saved = reopened
            .list_workflows()
            .unwrap()
            .into_iter()
            .find(|j| j.id == relaxed.id)
            .unwrap();
        assert_eq!(saved.review_similarity, Some(0.95));
        assert_eq!(saved.dedup_review_pairs.len(), 1);
        let mut job = reopened
            .new_workflow("dedup-auto", vec![second.id])
            .unwrap();
        job.review_similarity = Some(0.90);
        run(&mut reopened, &mut job, f.directory.path(), &cancel, |_| {}).unwrap();
        assert_eq!(reopened.list_candidates().unwrap()[0].state, "rejected");
        assert!(job.dedup_review_pairs.is_empty());
        assert_eq!(reopened.list_media().unwrap().len(), 2);
    }
    #[test]
    fn corrupt_or_incompatible_vectors_cannot_supply_a_duplicate_signal() {
        assert!(!valid_vector(&[1.0]));
        assert!(!valid_vector(&vec![1.0; 384]));
        assert!(!valid_vector(&vec![f32::NAN; 384]));
        let mut valid = vec![0.0; 384];
        valid[0] = 1.0;
        assert!(valid_vector(&valid));
    }
    #[test]
    fn phash_distance_two_always_shares_a_retrieval_bucket() {
        let hash = 0x63af127898ac4831;
        let keys = buckets(hash, None);
        for first in 0..64 {
            for second in first..64 {
                let nearby = hash ^ (1 << first) ^ (1 << second);
                assert!(buckets(nearby, None).iter().any(|key| keys.contains(key)));
            }
        }
    }
}
