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
            if job.export_layout == "flat" {
                let manifest = serde_json::json!({
                    "schema":"bdm-export-v1",
                    "media_id":id,
                    "image":image_name,
                    "sha256":media.sha256,
                    "source":media.post,
                    "current_dimensions":media.dimensions(),
                    "edit":media.edit,
                    "collections":media.collections,
                    "review":"keep",
                    "text_kind":job.mode,
                    "text_sha256":format!("{:x}",Sha256::digest(content.as_bytes())),
                    "layout":"flat"
                });
                write_flat_pair(
                    destination,
                    &base,
                    &image_name,
                    &original,
                    &content,
                    &manifest,
                )?;
                job.accepted.insert(id);
                job.cursor += 1;
                repo.save_workflow(job)?;
                report(job);
                continue;
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
            if job.export_layout == "flat" {
                write_training_config(&all, job, destination)?;
            }
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

fn write_training_config(
    media: &[bdm_domain::ingest::StoredMedia],
    job: &WorkflowJob,
    destination: &Path,
) -> Result<(), IngestError> {
    let Some(resolution) = media
        .iter()
        .filter(|m| job.accepted.contains(&m.id))
        .filter_map(|m| m.edit.as_ref()?.training.as_ref().map(|p| p.resolution))
        .max()
    else {
        return Ok(());
    };
    let mut dimensions = Vec::new();
    for entry in storage(fs::read_dir(destination))? {
        let path = storage(entry)?.path();
        if !path.is_file()
            || !path.extension().and_then(|v| v.to_str()).is_some_and(|v| {
                matches!(
                    v.to_ascii_lowercase().as_str(),
                    "jpg" | "jpeg" | "png" | "webp" | "bmp"
                )
            })
        {
            continue;
        }
        let (w, h) = storage(image::image_dimensions(&path))?;
        let plan = bdm_domain::training::plan(w, h, resolution).map_err(|reason| {
            IngestError::StorageDetails(format!("训练配置未生成：{}：{reason}", path.display()))
        })?;
        if plan.resized != [w, h] || plan.output != [w, h] {
            return Err(IngestError::StorageDetails(format!(
                "训练配置未生成：{} 尚未对齐 32／当前分辨率上限。",
                path.display()
            )));
        }
        dimensions.push([w, h]);
    }
    let image_dir = serde_json::to_string(&destination.to_string_lossy().replace('\\', "/"))
        .map_err(|_| IngestError::Storage)?;
    let max_bucket = dimensions
        .iter()
        .flatten()
        .copied()
        .max()
        .unwrap_or(resolution)
        .max(resolution);
    let config = format!("# Generated by Booru Dataset Manager; validated no-resize/no-crop geometry.\n[general]\ncaption_extension = \".txt\"\nshuffle_caption = false\n\n[[datasets]]\nresolution = [{resolution}, {resolution}]\nenable_bucket = true\nbucket_no_upscale = true\nbucket_reso_steps = 32\nmin_bucket_reso = 576\nmax_bucket_reso = {max_bucket}\n\n[[datasets.subsets]]\nimage_dir = {image_dir}\nnum_repeats = 1\nrandom_crop = false\nflip_aug = false\ncolor_aug = false\n");
    let config_path = destination.join("dataset.toml");
    let manifest_directory = destination.join(".bdm-manifests");
    storage(fs::create_dir_all(&manifest_directory))?;
    let manifest_path = manifest_directory.join("training-config.json");
    if config_path.exists() {
        let existing = storage(fs::read(&config_path))?;
        if existing != config.as_bytes() {
            let previous: serde_json::Value =
                storage(serde_json::from_slice(&storage(fs::read(&manifest_path))?))?;
            if previous["sha256"].as_str()
                != Some(format!("{:x}", Sha256::digest(&existing)).as_str())
            {
                return Err(IngestError::Integrity);
            }
        }
    }
    atomic_write(&config_path, config.as_bytes())?;
    let manifest = serde_json::json!({"schema":"bdm-training-32-v1","sha256":format!("{:x}",Sha256::digest(config.as_bytes())),"resolution":resolution,"dimensions":dimensions});
    atomic_write(
        &manifest_path,
        &storage(serde_json::to_vec_pretty(&manifest))?,
    )?;
    Ok(())
}

fn atomic_write(path: &Path, data: &[u8]) -> Result<(), IngestError> {
    let mut temp = storage(tempfile::NamedTempFile::new_in(
        path.parent().ok_or(IngestError::Storage)?,
    ))?;
    storage(temp.write_all(data))?;
    storage(temp.as_file().sync_all())?;
    storage(temp.persist(path))?;
    Ok(())
}

/// Writes training-friendly flat image/TXT pairs. Metadata lives under a hidden
/// directory so trainers see one matching .txt per image in the export root.
/// A changed TXT is replaced only when the last manifest proves the current
/// text was produced by us, so manual edits don't get silently overwritten.
fn write_flat_pair(
    root: &Path,
    base: &str,
    image_name: &str,
    image_bytes: &[u8],
    content: &str,
    manifest: &serde_json::Value,
) -> Result<(), IngestError> {
    let expected_image_sha = manifest["sha256"].as_str().ok_or(IngestError::Integrity)?;
    let mode = manifest["text_kind"]
        .as_str()
        .ok_or(IngestError::Integrity)?;
    let image_path = root.join(image_name);
    let text_path = root.join(format!("{base}.txt"));
    let manifest_dir = root.join(".bdm-manifests");
    storage(fs::create_dir_all(&manifest_dir))?;
    let manifest_path = manifest_dir.join(format!("{base}.json"));

    // Never overwrite an existing image with different bytes.
    if image_path.exists() {
        let old_image = storage(fs::read(&image_path))?;
        if format!("{:x}", Sha256::digest(old_image)) != expected_image_sha {
            return Err(IngestError::Integrity);
        }
    } else {
        let mut temp = storage(tempfile::NamedTempFile::new_in(root))?;
        storage(temp.write_all(image_bytes))?;
        storage(temp.as_file().sync_all())?;
        storage(temp.persist_noclobber(&image_path))?;
    }

    let prior_manifest = if manifest_path.is_file() {
        Some(storage(serde_json::from_slice::<serde_json::Value>(
            &storage(fs::read(&manifest_path))?,
        ))?)
    } else {
        None
    };
    if text_path.exists() {
        let old_text = storage(fs::read_to_string(&text_path))?;
        if old_text != content {
            let old_sha = format!("{:x}", Sha256::digest(old_text.as_bytes()));
            let trusted = prior_manifest.as_ref().is_some_and(|previous| {
                previous["text_sha256"].as_str() == Some(old_sha.as_str())
                    && previous["text_kind"].as_str() == Some(mode)
                    && previous["image"].as_str() == Some(image_name)
                    && previous["sha256"].as_str() == Some(expected_image_sha)
            });
            if !trusted {
                return Err(IngestError::Integrity);
            }
            atomic_write(&text_path, content.as_bytes())?;
        }
    } else {
        atomic_write(&text_path, content.as_bytes())?;
    }
    let manifest_bytes = storage(serde_json::to_vec(manifest))?;
    if prior_manifest.as_ref() != Some(manifest) {
        atomic_write(&manifest_path, &manifest_bytes)?;
    }
    Ok(())
}
