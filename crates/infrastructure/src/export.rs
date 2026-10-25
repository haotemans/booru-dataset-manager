use crate::{catalog::SqliteCatalog, ingest::storage, workflows::WorkflowJob};
use bdm_ports::IngestError;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub fn run(
    repo: &mut SqliteCatalog,
    job: &mut WorkflowJob,
    destination: &Path,
    cancel: &AtomicBool,
    mut report: impl FnMut(&WorkflowJob),
) -> Result<(), IngestError> {
    if !destination.is_absolute() || !matches!(job.mode.as_str(), "tags" | "caption") {
        return Err(IngestError::Storage);
    }
    storage(fs::create_dir_all(destination))?;
    let all = repo.list_media()?;
    let artifacts = repo.list_artifacts()?;
    let rules = repo.tag_rules()?;
    job.state = "running".into();
    repo.save_workflow(job)?;
    let result: Result<(), IngestError> = (|| {
        while job.cursor < job.ids.len() {
            if cancel.load(Ordering::Relaxed) {
                job.state = "paused".into();
                break;
            }
            let id = job.ids[job.cursor];
            let Some(media) = all.iter().find(|m| m.id == id) else {
                job.cursor += 1;
                continue;
            };
            let Some(text) = artifacts.iter().find(|a| {
                a.media_id == id
                    && a.kind == job.mode
                    && a.state == "complete"
                    && !a.content.trim().is_empty()
            }) else {
                job.cursor += 1;
                continue;
            };
            if !media.keep {
                job.cursor += 1;
                continue;
            }
            let content = if job.mode == "tags" {
                bdm_domain::tags::filter_character_tags(&text.content, true, &rules)
            } else {
                text.content.clone()
            };
            let base = format!("media_{id}");
            let directory = destination.join(&base);
            let image_name = format!("{base}.{}", media.post.file_ext);
            let original = storage(fs::read(&media.path))?;
            if format!("{:x}", Sha256::digest(&original)) != media.sha256 {
                return Err(IngestError::Integrity);
            }
            if directory.exists() {
                let existing = storage(fs::read(directory.join(&image_name)))?;
                let existing_text =
                    storage(fs::read_to_string(directory.join(format!("{base}.txt"))))?;
                if format!("{:x}", Sha256::digest(existing)) != media.sha256
                    || existing_text != content
                {
                    return Err(IngestError::Integrity);
                }
            } else {
                let temp = storage(
                    tempfile::Builder::new()
                        .prefix(".export-")
                        .tempdir_in(destination),
                )?;
                let files = [
                    (image_name.clone(), original),
                    (format!("{base}.txt"), content.as_bytes().to_vec()),
                ];
                for (name, bytes) in files {
                    let mut file = storage(fs::File::create(temp.path().join(name)))?;
                    storage(file.write_all(&bytes))?;
                    storage(file.sync_all())?;
                }
                let manifest = serde_json::json!({"schema":"bdm-export-v1","media_id":id,"image":image_name,"sha256":media.sha256,"source":media.post,"current_dimensions":media.dimensions(),"edit":media.edit,"collections":media.collections,"review":"keep","text_kind":job.mode,"text_sha256":format!("{:x}",Sha256::digest(content.as_bytes()))});
                let mut file = storage(fs::File::create(temp.path().join("manifest.json")))?;
                storage(file.write_all(storage(serde_json::to_string(&manifest))?.as_bytes()))?;
                storage(file.sync_all())?;
                drop(file);
                storage(fs::rename(temp.path(), &directory))?;
            }
            job.accepted.insert(id);
            job.cursor += 1;
            repo.save_workflow(job)?;
            report(job);
        }
        if job.cursor == job.ids.len() {
            job.state = "completed".into();
        }
        Ok(())
    })();
    if let Err(error) = &result {
        job.state = "failed".into();
        job.error = Some(error.to_string());
    }
    repo.save_workflow(job)?;
    report(job);
    result
}
