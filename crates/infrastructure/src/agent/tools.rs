use super::{params::*, state, Settings};
use crate::{ai, catalog::SqliteCatalog, editing, ingest::storage};
use base64::{engine::general_purpose::STANDARD, Engine};
use bdm_domain::{editing::CropRect, ingest::StoredMedia};
use bdm_ports::{CollectionRepository, IngestError};
use image::{DynamicImage, GenericImageView, ImageFormat};
use rmcp::model::{CallToolResult, ContentBlock};
use rusqlite::params;
use serde_json::{json, Value};
use sha2::Digest;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

fn fail(message: &str) -> IngestError {
    IngestError::StorageDetails(message.into())
}
pub(super) fn result(value: Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(value.to_string())])
}
fn writes(settings: &Settings) -> Result<(), IngestError> {
    if settings.writes {
        Ok(())
    } else {
        Err(fail(
            "MCP is read-only; this connection was not started with --allow-writes.",
        ))
    }
}
fn seen(settings: &Settings, sha: &str) -> Result<(), IngestError> {
    if settings
        .seen
        .lock()
        .map_err(|_| IngestError::Storage)?
        .contains(sha)
    {
        Ok(())
    } else {
        Err(fail("Call bdm_view_image on the full current image before deciding; IDs/tags alone are not visual inspection."))
    }
}
fn provenance(reason: &str, model: &str) -> Result<String, IngestError> {
    if reason.trim().is_empty()
        || reason.len() > 4096
        || model.trim().is_empty()
        || model.len() > 256
        || reason.contains('\0')
        || model.chars().any(char::is_control)
    {
        return Err(IngestError::InvalidMedia);
    }
    Ok(format!("mcp-antigravity:{model}"))
}
fn checked_path(path: &Path, settings: &Settings) -> Result<PathBuf, IngestError> {
    let path = storage(path.canonicalize())?;
    if !path.starts_with(&settings.library_root) {
        return Err(fail(
            "Stored image is outside this MCP server's configured library root.",
        ));
    }
    Ok(path)
}
fn media(
    repo: &SqliteCatalog,
    settings: &Settings,
    id: i64,
    expected: Option<&str>,
) -> Result<StoredMedia, IngestError> {
    let m = repo.media_by_id(id)?.ok_or(IngestError::InvalidMedia)?;
    checked_path(&m.path, settings)?;
    if expected.is_some_and(|sha| sha != m.sha256) || editing::hash(&m.path)? != m.sha256 {
        return Err(fail(
            "Image changed since inspection. Inspect again and use its fresh SHA-256.",
        ));
    }
    Ok(m)
}
fn original(m: &StoredMedia, settings: &Settings) -> Result<PathBuf, IngestError> {
    let downloaded = m
        .path
        .parent()
        .ok_or(IngestError::Storage)?
        .join(format!("downloaded-original.{}", m.post.file_ext));
    let path = if let Some(archive) = m.edit.as_ref().and_then(|e| e.source_archive.as_ref()) {
        if !archive.downloaded_original || editing::hash(&archive.path)? != archive.sha256 {
            return Err(IngestError::Integrity);
        }
        archive.path.clone()
    } else if downloaded.is_file() {
        downloaded
    } else {
        m.path.clone()
    };
    let path = checked_path(&path, settings)?;
    if format!("{:x}", md5::Md5::digest(storage(fs::read(&path))?)) != m.post.md5 {
        return Err(fail("A complete verified original is unavailable; the current edit is not being presented as an original."));
    }
    Ok(path)
}
fn info(repo: &SqliteCatalog, m: &StoredMedia) -> Result<Value, IngestError> {
    let artifacts=repo.list_artifacts()?.into_iter().filter(|a|a.media_id==m.id).map(|a|{
        let content=a.content.chars().take(1200).collect::<String>();
        json!({"kind":a.kind,"state":a.state,"reviewed":a.reviewed,"content":content,"content_truncated":content.len()<a.content.len(),"generated":a.is_generated_output()})
    }).collect::<Vec<_>>();
    let edit=m.edit.as_ref().map(|e|json!({"operation":e.operation,"crop":e.crop,"training":e.training,"dimensions":e.dimensions,"verified_original":e.source_archive.as_ref().is_some_and(|a|a.downloaded_original)}));
    Ok(
        json!({"id":m.id,"sha256":m.sha256,"dimensions":m.dimensions(),"original_dimensions":[m.post.image_width,m.post.image_height],"keep":m.keep,"artists":m.post.tag_string_artist,"characters":m.post.tag_string_character,"general_tags":m.post.tag_string_general.chars().take(1000).collect::<String>(),"current_edit":edit,"text":artifacts,"agent_review":review_summary(repo,m)?}),
    )
}
fn review_summary(repo: &SqliteCatalog, m: &StoredMedia) -> Result<Value, IngestError> {
    Ok(repo.agent_review_value(m.id)?.map(|r|json!({"decision":r["decision"],"current":r["image_sha256"]==m.sha256,"aesthetic":r["report"]["aesthetic"],"training_suitability":r["report"]["training_suitability"],"reason":r["report"]["reason"].as_str().unwrap_or("").chars().take(160).collect::<String>()})).unwrap_or(Value::Null))
}
fn page(mut rows: Vec<Value>, field: &str) -> CallToolResult {
    while rows.len() > 1 && serde_json::to_vec(&rows).is_ok_and(|b| b.len() > 7800) {
        rows.pop();
    }
    let next = rows.last().and_then(|r| r["id"].as_i64());
    result(json!({(field):rows,"next_after_id":next}))
}
fn picture(image: DynamicImage, mut data: Value) -> Result<CallToolResult, IngestError> {
    let native = image.dimensions();
    let display = image.thumbnail(1024, 1024);
    data["native_view_dimensions"] = json!(native);
    data["display_dimensions"] = json!(display.dimensions());
    let mut bytes = std::io::Cursor::new(Vec::new());
    storage(display.write_to(&mut bytes, ImageFormat::Png))?;
    Ok(CallToolResult::success(vec![
        ContentBlock::text(data.to_string()),
        ContentBlock::image(STANDARD.encode(bytes.into_inner()), "image/png"),
    ]))
}
fn rect(bbox: [u32; 4], dims: (u32, u32)) -> Result<CropRect, IngestError> {
    let [x1, y1, x2, y2] = bbox;
    if x1 >= x2 || y1 >= y2 || x2 > 1000 || y2 > 1000 {
        return Err(IngestError::InvalidMedia);
    }
    let x = (u64::from(dims.0) * u64::from(x1) / 1000) as u32;
    let y = (u64::from(dims.1) * u64::from(y1) / 1000) as u32;
    let right = (u64::from(dims.0) * u64::from(x2) / 1000) as u32;
    let bottom = (u64::from(dims.1) * u64::from(y2) / 1000) as u32;
    let crop = CropRect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    };
    if !crop.valid(dims) {
        return Err(IngestError::InvalidMedia);
    }
    Ok(crop)
}

pub(super) fn collections(
    repo: &mut SqliteCatalog,
    _: &Settings,
    args: ListArgs,
) -> Result<CallToolResult, IngestError> {
    let media = repo.list_media()?;
    let mut collections = repo.list().map_err(|_| IngestError::Storage)?;
    collections.sort_by_key(|c| c.id);
    let needle = args.collection.unwrap_or_default().to_lowercase();
    let rows = collections
        .into_iter()
        .filter(|c| c.id > args.after_id && c.name.to_lowercase().contains(&needle))
        .take(args.limit.unwrap_or(32).clamp(1, 64))
        .map(|c| {
            let count = media
                .iter()
                .filter(|m| m.collections.contains(&c.id))
                .count();
            let kind = match c.kind {
                bdm_domain::catalog::CollectionKind::Artist => "artist",
                bdm_domain::catalog::CollectionKind::Character => "character",
                bdm_domain::catalog::CollectionKind::Query => "query",
            };
            json!({"id":c.id,"tag":c.name,"kind":kind,"provider":c.provider,"images":count})
        })
        .collect::<Vec<_>>();
    Ok(page(rows, "collections"))
}
pub(super) fn list(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: ListArgs,
) -> Result<CallToolResult, IngestError> {
    let limit = args.limit.unwrap_or(12).clamp(1, 24);
    let collection = if let Some(tag) = args.collection {
        Some(
            repo.list()
                .map_err(|_| IngestError::Storage)?
                .into_iter()
                .filter(|c| c.name == tag)
                .map(|c| c.id)
                .collect::<BTreeSet<_>>(),
        )
    } else {
        None
    };
    let all = repo.list_media()?;
    let artifacts = repo.list_artifacts()?;
    let rows=all.iter().filter(|m|m.id>args.after_id && collection.as_ref().is_none_or(|ids|!ids.is_disjoint(&m.collections))).take(limit).map(|m|{
        let path_ok=checked_path(&m.path,settings).is_ok();
        Ok(json!({"id":m.id,"sha256":m.sha256,"dimensions":m.dimensions(),"keep":m.keep,"artist":m.post.tag_string_artist,"in_configured_library":path_ok,"caption_complete":artifacts.iter().any(|a|a.media_id==m.id&&a.kind=="caption"&&a.state=="complete"&&!a.content.is_empty()),"tags_generated":artifacts.iter().any(|a|a.media_id==m.id&&a.kind=="tags"&&a.is_generated_output()),"agent_review":review_summary(repo,m)?}))
    }).collect::<Result<Vec<_>,IngestError>>()?;
    Ok(page(rows, "images"))
}
pub(super) fn view(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: ViewArgs,
) -> Result<CallToolResult, IngestError> {
    let m = media(repo, settings, args.media_id, None)?;
    let path = if args.original {
        original(&m, settings)?
    } else {
        m.path.clone()
    };
    let mut image = storage(image::open(path))?;
    if let Some(bbox) = args.bbox_1000_xyxy {
        let r = rect(bbox, image.dimensions())?;
        image = image.crop_imm(r.x, r.y, r.width, r.height);
    } else {
        settings
            .seen
            .lock()
            .map_err(|_| IngestError::Storage)?
            .insert(if args.original {
                format!("original:{}", m.id)
            } else {
                m.sha256.clone()
            });
    }
    let mut data = info(repo, &m)?;
    data["view"] = json!(if args.original {
        "complete_original"
    } else {
        "current"
    });
    data["tile_bbox_1000_xyxy"] = json!(args.bbox_1000_xyxy);
    picture(image, data)
}
pub(super) fn crop(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: CropArgs,
) -> Result<CallToolResult, IngestError> {
    provenance(&args.reason, &args.model)?;
    let m = media(repo, settings, args.media_id, Some(&args.expected_sha256))?;
    seen(settings, &m.sha256)?;
    let selection = rect(args.bbox_1000_xyxy, m.dimensions())?;
    let key = (m.id, m.sha256.clone(), args.bbox_1000_xyxy);
    if args.apply {
        writes(settings)?;
        if !settings
            .previews
            .lock()
            .map_err(|_| IngestError::Storage)?
            .contains(&key)
        {
            return Err(fail("Preview this exact crop first with apply=false; then inspect the returned image before apply=true."));
        }
    }
    let preview = editing::native_crop_preview(&m, selection, &AtomicBool::new(false))?;
    if args.apply {
        let saved = repo.commit_repair(m.id, &preview)?;
        storage(repo.connection.execute(
            "UPDATE text_artifacts SET state='needs_review' WHERE media_id=?1 AND is_current=1",
            [m.id],
        ))?;
        repo.audit(
            "agent_crop",
            &json!({"id":m.id,"sha256":saved.sha256,"reason":args.reason,"model":args.model})
                .to_string(),
        )?;
        settings
            .previews
            .lock()
            .map_err(|_| IngestError::Storage)?
            .remove(&key);
        Ok(result(
            json!({"saved":true,"media":info(repo,&saved)?,"next":"Inspect the saved current image before writing its annotation."}),
        ))
    } else {
        settings
            .previews
            .lock()
            .map_err(|_| IngestError::Storage)?
            .insert(key);
        picture(
            storage(image::open(&preview.image))?,
            json!({"saved":false,"media_id":m.id,"expected_sha256":m.sha256,"selection":selection,"training":preview.training,"bbox_1000_xyxy":args.bbox_1000_xyxy}),
        )
    }
}
pub(super) fn restore(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: RestoreArgs,
) -> Result<CallToolResult, IngestError> {
    writes(settings)?;
    provenance(&args.reason, &args.model)?;
    let m = media(repo, settings, args.media_id, Some(&args.expected_sha256))?;
    original(&m, settings)?;
    if !settings
        .seen
        .lock()
        .map_err(|_| IngestError::Storage)?
        .contains(&format!("original:{}", m.id))
    {
        return Err(fail("Inspect the complete original first (original=true) before replacing the current edit."));
    }
    let saved = repo.restore_downloaded_original(m.id)?;
    repo.audit(
        "agent_restore",
        &json!({"id":m.id,"reason":args.reason,"model":args.model}).to_string(),
    )?;
    Ok(result(info(repo, &saved)?))
}

pub(super) fn annotate(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: AnnotationArgs,
) -> Result<CallToolResult, IngestError> {
    writes(settings)?;
    let source = provenance(&args.reason, &args.model)?;
    let lock = repo.media_lock.clone();
    let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
    let m = media(repo, settings, args.media_id, Some(&args.expected_sha256))?;
    seen(settings, &m.sha256)?;
    if args.caption.is_none() && args.tags.is_none() {
        return Err(IngestError::InvalidMedia);
    }
    let mut rules = repo.tag_rules()?;
    for tag in m.post.tag_string_general.split_whitespace() {
        rules.categories.entry(tag.into()).or_insert(0);
    }
    let normalized = bdm_domain::tags::normalize(
        &m.post.tag_string,
        &m.post.tag_string_artist,
        &m.post.tag_string_character,
        &rules,
    );
    let mut outputs = Vec::new();
    if let Some(caption) = &args.caption {
        if caption.trim().is_empty()
            || caption.len() > 8192
            || caption.chars().any(char::is_control)
            || ai::identity::caption_mentions_identity(
                caption,
                &normalized,
                &rules,
                &m.post.tag_string_copyright,
            )
        {
            return Err(fail("Caption must be visual prose without artist/character identity prefixes; put exact visible signature text in text_regions."));
        }
        outputs.push((
            "caption",
            ai::format_caption(caption, &normalized, "at", "plain"),
        ));
    }
    let mut rejected = Vec::new();
    if let Some(tags) = &args.tags {
        if tags.is_empty() || tags.len() > 512 || tags.iter().any(|t| t.len() > 256) {
            return Err(IngestError::InvalidMedia);
        }
        let verified = ai::identity::verified_visual_tags(tags, &rules);
        for tag in tags {
            if ai::identity::verified_visual_tags(std::slice::from_ref(tag), &rules).is_empty() {
                rejected.push(tag.clone());
            }
        }
        if verified.is_empty() {
            return Err(fail("No locally verified general tags survived. Use known general-category terms; identities are merged locally."));
        }
        outputs.push((
            "tags",
            ai::format_merged_tags(&verified, &normalized, "at", "plain"),
        ));
    }
    let regions = args
        .text_regions
        .unwrap_or_default()
        .into_iter()
        .map(|r| ai::TextRegion {
            text: r.text,
            bbox_1000_xyxy: r.bbox_1000_xyxy,
            kind: r.kind,
            legibility: r.legibility,
        })
        .collect::<Vec<_>>();
    let special = args.special_elements.unwrap_or_default();
    if regions.len() > 64
        || regions.iter().any(|r| !r.is_valid())
        || special
            .iter()
            .any(|e| !matches!(e.as_str(), "x_ray" | "cross_section"))
        || special.len() > 2
    {
        return Err(IngestError::InvalidMedia);
    }
    let existing = repo.list_artifacts()?;
    let mut saved = Vec::new();
    let mut preserved = Vec::new();
    for (kind, content) in outputs {
        let protected = existing.iter().any(|a| {
            a.media_id == m.id && a.kind == kind && (a.is_generated_output() || a.reviewed)
        });
        let path = m
            .path
            .with_extension(if kind == "tags" { "tags.txt" } else { "txt" });
        let manual = match fs::read(&path) {
            Ok(bytes) => {
                !bytes.iter().all(u8::is_ascii_whitespace)
                    && !existing.iter().any(|a| {
                        a.media_id == m.id && a.kind == kind && a.content.as_bytes() == bytes
                    })
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(IngestError::StorageDetails(error.to_string())),
        };
        if !args.overwrite && (protected || manual) {
            preserved.push(kind);
            continue;
        }
        repo.write_artifact_locked(&m, kind, &content, &source, "complete", true)?;
        saved.push(kind);
    }
    if !saved.is_empty() {
        repo.save_annotation_details(
            m.id,
            &ai::AnnotationDetails {
                schema_version: "agent-v1".into(),
                text_regions: regions,
                special_elements: special,
                user_hint: Some(args.reason.clone()),
            },
        )?;
    }
    repo.audit("agent_annotation",&json!({"id":m.id,"sha256":m.sha256,"saved":saved,"preserved":preserved,"model":args.model,"reason":args.reason}).to_string())?;
    Ok(result(
        json!({"media_id":m.id,"saved":saved,"preserved":preserved,"rejected_tags":rejected}),
    ))
}

pub(super) fn assess(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: AssessArgs,
) -> Result<CallToolResult, IngestError> {
    writes(settings)?;
    provenance(&args.reason, &args.model)?;
    if [
        args.aesthetic,
        args.training_suitability,
        args.composition,
        args.detail,
    ]
    .into_iter()
    .flatten()
    .any(|v| !v.is_finite() || !(0.0..=10.0).contains(&v))
        || args.style.len() > 256
    {
        return Err(IngestError::InvalidMedia);
    }
    let lock = repo.media_lock.clone();
    let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
    let m = media(repo, settings, args.media_id, Some(&args.expected_sha256))?;
    seen(settings, &m.sha256)?;
    let decision = match args.decision {
        Decision::Keep => "keep",
        Decision::Reject => "reject",
        Decision::Defer => "defer",
    };
    let report = storage(serde_json::to_value(&args))?;
    state::save_review(repo, m.id, &m.sha256, decision, &report, None)?;
    storage(repo.connection.execute("INSERT INTO media_quality(media_id,aesthetic,watermark_free,provenance) VALUES(?1,?2,?3,'{}') ON CONFLICT(media_id) DO UPDATE SET aesthetic=excluded.aesthetic,watermark_free=excluded.watermark_free",params![m.id,args.aesthetic,args.watermark_present.map(|v|!v)]))?;
    if matches!(args.decision, Decision::Keep | Decision::Defer) {
        storage(repo.connection.execute(
            "UPDATE media SET keep=?2 WHERE id=?1",
            params![m.id, matches!(args.decision, Decision::Keep)],
        ))?;
    }
    repo.audit("agent_assessment", &report.to_string())?;
    Ok(result(
        json!({"id":m.id,"decision":decision,"queued_for_deletion":decision=="reject","files_deleted":0}),
    ))
}

pub(super) fn scan(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: ScanArgs,
) -> Result<CallToolResult, IngestError> {
    writes(settings)?;
    let ids = args.media_ids.into_iter().collect::<BTreeSet<_>>();
    if ids.len() < 2
        || ids.len() > 64
        || args
            .similarity
            .is_some_and(|s| !s.is_finite() || !(0.85..=1.0).contains(&s))
    {
        return Err(IngestError::InvalidMedia);
    }
    for id in &ids {
        media(repo, settings, *id, None)?;
    }
    // Agent scans are analysis-only; serialize image mutations while features
    // are being computed so their hashes cannot silently become stale.
    let lock = repo.media_lock.clone();
    let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
    let mut job = repo.new_workflow("agent-dedup", ids.iter().copied().collect())?;
    job.dismissed = true;
    job.review_similarity = Some(args.similarity.unwrap_or(0.95));
    crate::visual::run(
        repo,
        &mut job,
        &settings.project_root.join("models/dinov3"),
        &AtomicBool::new(false),
        |_| {},
    )?;
    let candidates = repo
        .list_candidates()?
        .into_iter()
        .filter(|p| p.state == "pending" && ids.contains(&p.first) && ids.contains(&p.second))
        .collect::<Vec<_>>();
    repo.audit(
        "agent_duplicate_scan",
        &json!({"job":job.id,"ids":ids,"state":job.state}).to_string(),
    )?;
    let count = candidates.len();
    Ok(result(
        json!({"state":job.state,"error":job.error,"candidate_count":count,"candidates":candidates.into_iter().take(24).collect::<Vec<_>>(),"more_pending":count>24,"files_deleted":0,"note":"View both current images before resolving. Resolve returned pairs and rescan for remaining pending pairs. Missing DINOv3 is not successful visual analysis."}),
    ))
}

pub(super) fn pair(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: PairArgs,
) -> Result<CallToolResult, IngestError> {
    writes(settings)?;
    provenance(&args.reason, &args.model)?;
    if args.first_id == args.second_id
        || args
            .survivor_id
            .is_some_and(|id| id != args.first_id && id != args.second_id)
    {
        return Err(IngestError::InvalidMedia);
    }
    let lock = repo.media_lock.clone();
    let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
    let first = media(
        repo,
        settings,
        args.first_id,
        Some(&args.expected_sha256[0]),
    )?;
    let second = media(
        repo,
        settings,
        args.second_id,
        Some(&args.expected_sha256[1]),
    )?;
    seen(settings, &first.sha256)?;
    seen(settings, &second.sha256)?;
    let report = json!({"first_id":first.id,"second_id":second.id,"survivor_id":args.survivor_id,"reason":args.reason,"model":args.model});
    if let Some(id) = args.survivor_id {
        let (keep, reject) = if first.id == id {
            (&first, &second)
        } else {
            (&second, &first)
        };
        state::save_review(
            repo,
            reject.id,
            &reject.sha256,
            "reject",
            &report,
            Some((keep.id, &keep.sha256)),
        )?;
        state::save_review(repo, keep.id, &keep.sha256, "keep", &report, None)?;
        storage(
            repo.connection
                .execute("UPDATE media SET keep=1 WHERE id=?1", [keep.id]),
        )?;
    } else {
        let mut candidate = repo
            .list_candidates()?
            .into_iter()
            .find(|p| p.first == first.id.min(second.id) && p.second == first.id.max(second.id))
            .unwrap_or(bdm_domain::dedup::DuplicateCandidate {
                first: first.id.min(second.id),
                second: first.id.max(second.id),
                phash_distance: None,
                cosine: None,
                model_version: None,
                state: "pending".into(),
            });
        candidate.state = "rejected".into();
        repo.save_candidate(&candidate)?;
        for m in [&first, &second] {
            state::save_review(repo, m.id, &m.sha256, "keep", &report, None)?;
        }
    }
    // Suppress automatic deletion by the ordinary dedup worker while the AI's
    // chosen removal is still awaiting final deletion review.
    let candidate = bdm_domain::dedup::DuplicateCandidate {
        first: first.id.min(second.id),
        second: first.id.max(second.id),
        phash_distance: None,
        cosine: None,
        model_version: None,
        state: "rejected".into(),
    };
    repo.save_candidate(&candidate)?;
    repo.audit("agent_duplicate_decision", &report.to_string())?;
    Ok(result(json!({"decision":report,"files_deleted":0})))
}
