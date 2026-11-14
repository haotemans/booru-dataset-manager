use crate::{catalog::SqliteCatalog, ingest::storage, workflows::WorkflowJob};
use bdm_ports::IngestError;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(test)]
mod tests;

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
        if job.export_layout == "multi-resolution" {
            for entry in storage(fs::read_dir(destination))? {
                let path = storage(entry)?.path();
                if path.is_file() && is_image_path(&path) {
                    return Err(IngestError::StorageDetails(
                        "多分辨率副本请使用独立目录，避免与原分辨率训练集混合。".into(),
                    ));
                }
            }
        } else if job.export_layout == "flat"
            && bdm_domain::training::RESOLUTIONS
                .into_iter()
                .any(|resolution| {
                    destination
                        .join(resolution.to_string())
                        .join(".bdm-manifests/training-config.json")
                        .is_file()
                })
        {
            return Err(IngestError::StorageDetails(
                "此目录已有多分辨率副本，请为原分辨率扁平导出选择独立目录。".into(),
            ));
        }
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
            let image_name = format!("{base}.{}", media.current_extension());
            let original = storage(fs::read(&media.path))?;
            if format!("{:x}", Sha256::digest(&original)) != media.sha256 {
                return Err(IngestError::Integrity);
            }
            if matches!(job.export_layout.as_str(), "flat" | "multi-resolution") {
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
                if job.export_layout == "multi-resolution" {
                    let image = storage(image::load_from_memory(&original))?;
                    let mut exported = false;
                    for resolution in bdm_domain::training::RESOLUTIONS {
                        let Ok(plan) =
                            bdm_domain::training::plan(image.width(), image.height(), resolution)
                        else {
                            continue;
                        };
                        let bytes = training_variant(&image, &plan)?;
                        let variant_name = format!("{base}.png");
                        let variant_root = destination.join(resolution.to_string());
                        storage(fs::create_dir_all(&variant_root))?;
                        let mut variant = manifest.clone();
                        variant["source_sha256"] = media.sha256.clone().into();
                        variant["image"] = variant_name.clone().into();
                        variant["sha256"] = format!("{:x}", Sha256::digest(&bytes)).into();
                        variant["current_dimensions"] = serde_json::json!(plan.output);
                        variant["training"] = serde_json::json!(plan);
                        variant["layout"] = "multi-resolution".into();
                        write_flat_pair(
                            &variant_root,
                            &base,
                            &variant_name,
                            &bytes,
                            &content,
                            &variant,
                        )?;
                        exported = true;
                    }
                    if !exported {
                        return Err(IngestError::StorageDetails(format!(
                            "图片 {id} 不满足任何训练档位的尺寸下限；原文件未修改。"
                        )));
                    }
                } else {
                    // Default export is an exact copy, independent of automatic
                    // crop tier and the resolution used by the eventual trainer.
                    write_flat_pair(
                        destination,
                        &base,
                        &image_name,
                        &original,
                        &content,
                        &manifest,
                    )?;
                }
                job.accepted.insert(id);
                job.cursor += 1;
                repo.save_workflow(job)?;
                report(job);
                continue;
            }
            let manifest = serde_json::json!({"schema":"bdm-export-v1","media_id":id,"image":image_name,"sha256":media.sha256,"source":media.post,"current_dimensions":media.dimensions(),"edit":media.edit,"collections":media.collections,"review":"keep","text_kind":job.mode,"text_sha256":format!("{:x}",Sha256::digest(content.as_bytes()))});
            storage(fs::create_dir_all(&directory))?;
            write_managed_pair(
                &directory,
                &base,
                &image_name,
                &original,
                &content,
                &manifest,
                &directory.join("manifest.json"),
            )?;
            job.accepted.insert(id);
            job.cursor += 1;
            repo.save_workflow(job)?;
            report(job);
        }
        if job.cursor == job.ids.len() {
            if job.export_layout == "flat" && !job.accepted.is_empty() {
                write_training_config(destination, 1024, false)?;
            } else if job.export_layout == "multi-resolution" {
                for resolution in bdm_domain::training::RESOLUTIONS {
                    let variant = destination.join(resolution.to_string());
                    if variant.is_dir() {
                        write_training_config(&variant, resolution, true)?;
                    }
                }
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
    destination: &Path,
    resolution: u32,
    fixed_geometry: bool,
) -> Result<(), IngestError> {
    let mut dimensions = Vec::new();
    for entry in storage(fs::read_dir(destination))? {
        let path = storage(entry)?.path();
        if !path.is_file() || !is_image_path(&path) {
            continue;
        }
        let (w, h) = storage(image::image_dimensions(&path))?;
        if fixed_geometry {
            let plan = bdm_domain::training::plan(w, h, resolution).map_err(|reason| {
                IngestError::StorageDetails(format!("训练配置未生成：{}：{reason}", path.display()))
            })?;
            if plan.resized != [w, h] || plan.output != [w, h] {
                return Err(IngestError::StorageDetails(format!(
                    "训练配置未生成：{} 尚未对齐 32／当前分辨率上限。",
                    path.display()
                )));
            }
        }
        dimensions.push([w, h]);
    }
    if dimensions.is_empty() {
        return Ok(());
    }
    dimensions.sort_unstable();
    let image_dir = serde_json::to_string(&destination.to_string_lossy().replace('\\', "/"))
        .map_err(|_| IngestError::Storage)?;
    let max_bucket = dimensions
        .iter()
        .flatten()
        .copied()
        .max()
        .unwrap_or(resolution)
        .max(resolution);
    let policy = if fixed_geometry {
        "Validated training variants; no further geometry change with this config."
    } else {
        "Native-size flat export. This 1024 training reference may resize/crop in the trainer; exported images are unchanged."
    };
    let config = format!("# Generated by Booru Dataset Manager.\n# {policy}\n[general]\ncaption_extension = \".txt\"\nshuffle_caption = false\n\n[[datasets]]\nresolution = [{resolution}, {resolution}]\nenable_bucket = true\nbucket_no_upscale = true\nbucket_reso_steps = 32\nmin_bucket_reso = 576\nmax_bucket_reso = {max_bucket}\n\n[[datasets.subsets]]\nimage_dir = {image_dir}\nnum_repeats = 1\nrandom_crop = false\nflip_aug = false\ncolor_aug = false\n");
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
    let manifest = serde_json::json!({"schema":"bdm-training-32-v1","sha256":format!("{:x}",Sha256::digest(config.as_bytes())),"resolution":resolution,"dimensions":dimensions,"training_may_resize":!fixed_geometry});
    atomic_write(
        &manifest_path,
        &storage(serde_json::to_vec_pretty(&manifest))?,
    )?;
    Ok(())
}

fn is_image_path(path: &Path) -> bool {
    path.extension().and_then(|v| v.to_str()).is_some_and(|v| {
        matches!(
            v.to_ascii_lowercase().as_str(),
            "jpg" | "jpeg" | "png" | "webp" | "bmp"
        )
    })
}

fn training_variant(
    image: &image::DynamicImage,
    plan: &bdm_domain::training::TrainingCrop,
) -> Result<Vec<u8>, IngestError> {
    let resized = if [image.width(), image.height()] == plan.resized {
        image.clone()
    } else {
        image.resize_exact(
            plan.resized[0],
            plan.resized[1],
            image::imageops::FilterType::Lanczos3,
        )
    };
    let output = resized.crop_imm(
        plan.offset[0],
        plan.offset[1],
        plan.output[0],
        plan.output[1],
    );
    let mut bytes = std::io::Cursor::new(Vec::new());
    storage(output.write_to(&mut bytes, image::ImageFormat::Png))?;
    Ok(bytes.into_inner())
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

/// Flat exports contain one current image/TXT pair; existing text is immutable.
fn write_flat_pair(
    root: &Path,
    base: &str,
    image_name: &str,
    image_bytes: &[u8],
    content: &str,
    manifest: &serde_json::Value,
) -> Result<(), IngestError> {
    let manifest_dir = root.join(".bdm-manifests");
    storage(fs::create_dir_all(&manifest_dir))?;
    write_managed_pair(
        root,
        base,
        image_name,
        image_bytes,
        content,
        manifest,
        &manifest_dir.join(format!("{base}.json")),
    )
}

fn valid_image_name(base: &str, name: &str) -> bool {
    ["png", "jpg", "jpeg", "webp"]
        .into_iter()
        .any(|ext| name == format!("{base}.{ext}"))
}

fn file_sha(path: &Path) -> Result<String, IngestError> {
    Ok(format!("{:x}", Sha256::digest(storage(fs::read(path))?)))
}

fn cleanup_export_images(
    root: &Path,
    base: &str,
    previous: Option<&serde_json::Value>,
    next: &serde_json::Value,
) -> Result<(), IngestError> {
    let name = next["image"].as_str().ok_or(IngestError::Integrity)?;
    if !valid_image_name(base, name)
        || file_sha(&root.join(name))? != next["sha256"].as_str().ok_or(IngestError::Integrity)?
    {
        return Err(IngestError::Integrity);
    }
    if let Some(old) = previous {
        let old_name = old["image"].as_str().ok_or(IngestError::Integrity)?;
        if !valid_image_name(base, old_name)
            || old["media_id"] != next["media_id"]
            || old["source"] != next["source"]
        {
            return Err(IngestError::Integrity);
        }
        let old_path = root.join(old_name);
        if old_name != name && old_path.is_file() {
            if file_sha(&old_path)? != old["sha256"].as_str().ok_or(IngestError::Integrity)? {
                return Err(IngestError::Integrity);
            }
            storage(fs::remove_file(old_path))?;
        }
    }
    // Legacy backups are read only as cleanup evidence. Never create new ones.
    let backups = root.join(".bdm-manifests/replaced-images");
    if backups.is_dir() {
        for entry in storage(fs::read_dir(&backups))? {
            let entry = storage(entry)?;
            if !storage(entry.file_type())?.is_file() {
                continue;
            }
            let filename = entry.file_name();
            let Some((sha, image)) = filename.to_str().and_then(|v| v.split_once('-')) else {
                continue;
            };
            let Some(image) = image.strip_suffix(".bak") else {
                continue;
            };
            if sha.len() == 64
                && sha.bytes().all(|v| v.is_ascii_hexdigit())
                && valid_image_name(base, image)
            {
                if file_sha(&entry.path())? != sha {
                    return Err(IngestError::Integrity);
                }
                storage(fs::remove_file(entry.path()))?;
            }
        }
        if storage(fs::read_dir(&backups))?.next().is_none() {
            storage(fs::remove_dir(backups))?;
        }
    }
    Ok(())
}

/// A metadata intent makes image replacement restartable without storing old
/// image bytes. It covers both same-name PNG updates and JPEG->PNG migration.
fn recover_export_update(
    root: &Path,
    base: &str,
    manifest_path: &Path,
    intent: &Path,
) -> Result<(), IngestError> {
    if !intent.is_file() {
        return Ok(());
    }
    let pending: serde_json::Value = storage(serde_json::from_slice(&storage(fs::read(intent))?))?;
    let old = &pending["previous"];
    let mut next = pending["next"].clone();
    let old_name = old["image"].as_str().ok_or(IngestError::Integrity)?;
    let next_name = next["image"].as_str().ok_or(IngestError::Integrity)?;
    if !valid_image_name(base, old_name)
        || !valid_image_name(base, next_name)
        || old["media_id"] != next["media_id"]
        || old["source"] != next["source"]
    {
        return Err(IngestError::Integrity);
    }
    let existing: serde_json::Value =
        storage(serde_json::from_slice(&storage(fs::read(manifest_path))?))?;
    let same_image = |receipt: &serde_json::Value| {
        ["media_id", "source", "image", "sha256"]
            .into_iter()
            .all(|key| existing[key] == receipt[key])
    };
    if !same_image(old) && !same_image(&next) {
        return Err(IngestError::Integrity);
    }
    let new_path = root.join(next_name);
    if new_path.is_file()
        && file_sha(&new_path)? == next["sha256"].as_str().ok_or(IngestError::Integrity)?
    {
        cleanup_export_images(root, base, Some(old), &next)?;
        let text = root.join(format!("{base}.txt"));
        if text.is_file() {
            next["text_sha256"] = file_sha(&text)?.into();
        }
        atomic_write(manifest_path, &storage(serde_json::to_vec(&next))?)?;
    } else {
        // No image promotion occurred: discard only metadata and retry.
        if file_sha(&root.join(old_name))?
            != old["sha256"].as_str().ok_or(IngestError::Integrity)?
            || (old_name != next_name && new_path.exists())
        {
            return Err(IngestError::Integrity);
        }
    }
    storage(fs::remove_file(intent))?;
    Ok(())
}

fn write_managed_pair(
    root: &Path,
    base: &str,
    image_name: &str,
    image_bytes: &[u8],
    content: &str,
    manifest: &serde_json::Value,
    manifest_path: &Path,
) -> Result<(), IngestError> {
    let expected_image_sha = manifest["sha256"].as_str().ok_or(IngestError::Integrity)?;
    if !valid_image_name(base, image_name)
        || format!("{:x}", Sha256::digest(image_bytes)) != expected_image_sha
    {
        return Err(IngestError::Integrity);
    }
    let metadata_dir = root.join(".bdm-manifests");
    storage(fs::create_dir_all(&metadata_dir))?;
    let intent = metadata_dir.join(format!("{base}.image-update.json"));
    recover_export_update(root, base, manifest_path, &intent)?;
    let image_path = root.join(image_name);
    let text_path = root.join(format!("{base}.txt"));
    let prior_manifest = if manifest_path.is_file() {
        Some(storage(serde_json::from_slice::<serde_json::Value>(
            &storage(fs::read(manifest_path))?,
        ))?)
    } else {
        None
    };
    if let Some(previous) = &prior_manifest {
        let old_name = previous["image"].as_str().ok_or(IngestError::Integrity)?;
        if !valid_image_name(base, old_name)
            || previous["media_id"] != manifest["media_id"]
            || previous["source"] != manifest["source"]
        {
            return Err(IngestError::Integrity);
        }
        if old_name != image_name
            && root.join(old_name).is_file()
            && file_sha(&root.join(old_name))?
                != previous["sha256"].as_str().ok_or(IngestError::Integrity)?
        {
            return Err(IngestError::Integrity);
        }
    }
    for ext in ["png", "jpg", "jpeg", "webp"] {
        let name = format!("{base}.{ext}");
        if name != image_name
            && root.join(&name).exists()
            && !prior_manifest
                .as_ref()
                .is_some_and(|p| p["image"].as_str() == Some(name.as_str()))
        {
            return Err(IngestError::Integrity);
        }
    }
    let existing_sha = if image_path.is_file() {
        Some(file_sha(&image_path)?)
    } else {
        None
    };
    if let Some(sha) = &existing_sha {
        if sha != expected_image_sha
            && !prior_manifest.as_ref().is_some_and(|p| {
                p["image"].as_str() == Some(image_name)
                    && p["sha256"].as_str() == Some(sha.as_str())
            })
        {
            return Err(IngestError::Integrity);
        }
    }
    let mut next = manifest.clone();
    if text_path.exists() {
        next["text_sha256"] = file_sha(&text_path)?.into();
        next["text_preserved"] = true.into();
        if let Some(prior) = &prior_manifest {
            next["text_kind"] = prior["text_kind"].clone();
        }
    } else {
        atomic_write(&text_path, content.as_bytes())?;
    }
    if let Some(previous) = &prior_manifest {
        atomic_write(
            &intent,
            &storage(serde_json::to_vec(
                &serde_json::json!({"previous":previous,"next":next}),
            ))?,
        )?;
    }
    if existing_sha.as_deref() != Some(expected_image_sha) {
        let mut temp = storage(tempfile::NamedTempFile::new_in(root))?;
        storage(temp.write_all(image_bytes))?;
        storage(temp.as_file().sync_all())?;
        if existing_sha.is_some() {
            if file_sha(&image_path)?.as_str()
                != existing_sha.as_deref().ok_or(IngestError::Integrity)?
            {
                return Err(IngestError::Integrity);
            }
            storage(temp.persist(&image_path).map_err(|e| e.error))?;
        } else {
            storage(temp.persist_noclobber(&image_path).map_err(|e| e.error))?;
        }
    }
    cleanup_export_images(root, base, prior_manifest.as_ref(), &next)?;
    if prior_manifest.as_ref() != Some(&next) {
        atomic_write(manifest_path, &storage(serde_json::to_vec(&next))?)?;
    }
    if intent.is_file() {
        storage(fs::remove_file(intent))?;
    }
    Ok(())
}
