use super::service::ServiceReply;
use super::{params::*, state, Settings};
use crate::{ai, catalog::SqliteCatalog, editing, ingest::storage};
use bdm_domain::{editing::CropRect, ingest::StoredMedia};
use bdm_ports::IngestError;
use image::{DynamicImage, GenericImageView, ImageFormat};
use rusqlite::{params, OptionalExtension};
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
pub(super) fn result(value: Value) -> ServiceReply {
    ServiceReply::success(value)
}
fn guidance_error(code: &str, message: &str, next: Value) -> IngestError {
    fail(&json!({"status":"error","code":code,"message":message,"next":next}).to_string())
}
pub(super) fn error_result(error: IngestError) -> ServiceReply {
    let mut value = match &error {
        IngestError::StorageDetails(details) => serde_json::from_str::<Value>(details)
            .ok().filter(|v| v["code"].is_string()),
        _ => None,
    }.unwrap_or_else(|| json!({"status":"error","code":"tool_failed","message":error.to_string(),
        "next":{"action":"correct_request_or_report","hint":"Check tool schema and the error. Do not repeat unchanged inputs or start another workflow."}}));
    value["status"] = json!("error");
    ServiceReply::error(value)
}
fn writes(settings: &Settings) -> Result<(), IngestError> {
    if settings.writes {
        Ok(())
    } else {
        Err(guidance_error(
            "read_only",
            "MCP is read-only; this connection was not started with --allow-writes.",
            json!({"action":"report_read_only","hint":"Writing requires restarting this server with --allow-writes; no retry can enable it."}),
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
        Err(guidance_error(
            "image_not_viewed",
            "View the full current image before deciding.",
            json!({"tool":"bdm_view_image","args":{"original":false},"hint":"Use the target media_id. A detail tile alone is insufficient."}),
        ))
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
        return Err(guidance_error("invalid_provenance", "Supply a nonempty reason (<=4096 bytes) and actual model identifier (<=256 bytes, or unknown).",
            json!({"action":"correct_request","fields":["reason","model"]})));
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
pub(super) fn media(
    repo: &SqliteCatalog,
    settings: &Settings,
    id: i64,
    expected: Option<&str>,
) -> Result<StoredMedia, IngestError> {
    let m = repo.media_by_id(id)?.ok_or_else(|| {
        guidance_error(
            "media_not_found",
            "No image with this media_id.",
            json!({"tool":"bdm_list_media","hint":"Use an ID from the requested collection."}),
        )
    })?;
    checked_path(&m.path, settings)?;
    if expected.is_some() {
        let exists: bool = storage(repo.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='agent_work')",
            [],
            |r| r.get(0),
        ))?;
        if exists {
            let active:Option<(i64,String,i64)>=storage(repo.connection.query_row("SELECT id,lease,generation FROM agent_work WHERE media_id=?1 AND lease IS NOT NULL AND expires>?2",params![id,super::jobs::now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional())?;
            if active.is_some() && active != settings.permit {
                return Err(guidance_error(
                    "media_leased",
                    "This image is owned by an active work lease; submit through that work item.",
                    json!({"operation":"work_claim","hint":"Do not bypass the task plan with standalone mutation tools."}),
                ));
            }
            if let Some((work_id, _, _)) = &settings.permit {
                let revision: String = storage(repo.connection.query_row(
                    "SELECT revision FROM agent_work WHERE id=?1",
                    [work_id],
                    |r| r.get(0),
                ))?;
                if revision != super::jobs::annotation_revision(repo, &m)? {
                    return Err(guidance_error("annotation_conflict","Annotation or sidecar changed after planning; preserve the newer text and inspect it before submitting a new plan.",json!({"action":"replan_after_review"})));
                }
            }
        }
    }
    if expected.is_some_and(|sha| sha != m.sha256) || editing::hash(&m.path)? != m.sha256 {
        return Err(guidance_error(
            "stale_image",
            "Image changed since inspection. Inspect again and use its fresh SHA-256.",
            json!({"tool":"bdm_view_image","args":{"media_id":id,"original":false}}),
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
fn artifacts(
    repo: &SqliteCatalog,
    id: i64,
) -> Result<Vec<crate::workflows::TextArtifact>, IngestError> {
    let mut query = storage(repo.connection.prepare("SELECT kind,content,state,reviewed,provenance FROM text_artifacts WHERE media_id=?1 AND is_current=1"))?;
    storage(
        query
            .query_map([id], |r| {
                Ok(crate::workflows::TextArtifact {
                    media_id: id,
                    kind: r.get(0)?,
                    content: r.get(1)?,
                    state: r.get(2)?,
                    reviewed: r.get(3)?,
                    provenance: r.get(4)?,
                })
            })
            .and_then(|rows| rows.collect()),
    )
}
pub(super) fn annotation_details(
    repo: &SqliteCatalog,
    id: i64,
) -> Result<ai::AnnotationDetails, IngestError> {
    let raw: Option<String> = storage(
        repo.connection
            .query_row(
                "SELECT provenance FROM media_quality WHERE media_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional(),
    )?;
    Ok(raw
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default())
}
fn tag_rules(
    repo: &SqliteCatalog,
    m: &StoredMedia,
) -> Result<bdm_domain::tags::TagRules, IngestError> {
    let mut rules = repo.tag_rules()?;
    for tag in m.post.tag_string_general.split_whitespace() {
        rules.categories.entry(tag.into()).or_insert(0);
    }
    Ok(rules)
}
fn tag_vocabulary(m: &StoredMedia, rules: &bdm_domain::tags::TagRules) -> Value {
    let tags = ai::identity::verified_visual_tags(
        &m.post
            .tag_string_general
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        rules,
    );
    json!({"source_general":tags,"rule":"Choose only visually supported terms. Other locally verified category-0 terms are accepted; unknown terms belong in caption prose, not forced into tags."})
}
/// Omission is not deletion permission. Source terms protect an unedited image;
/// edited pixels require a tag result explicitly bound to that current image.
/// Never auto-append terms: the client still has to inspect the pixels.
fn required_visual_tags(
    m: &StoredMedia,
    rules: &bdm_domain::tags::TagRules,
    existing: &[crate::workflows::TextArtifact],
    details: &ai::AnnotationDetails,
) -> Vec<String> {
    let mut candidates = Vec::new();
    if m.edit.is_none() {
        candidates.extend(
            m.post
                .tag_string_general
                .split_whitespace()
                .map(str::to_owned),
        );
    }
    for artifact in existing.iter().filter(|a| a.kind == "tags") {
        let bound_to_current = details.tags_image_sha256.as_deref() == Some(&m.sha256)
            && details.tags_content_sha256.as_deref()
                == Some(
                    format!("{:x}", sha2::Sha256::digest(artifact.content.as_bytes())).as_str(),
                );
        if m.edit.is_none() || bound_to_current {
            candidates.extend(artifact.content.split(',').map(|t| t.trim().to_owned()));
        }
    }
    ai::identity::verified_visual_tags(&candidates, rules)
}
fn info(repo: &SqliteCatalog, m: &StoredMedia) -> Result<Value, IngestError> {
    let artifacts=artifacts(repo,m.id)?.into_iter().map(|a|{
        let content=a.content.chars().take(1200).collect::<String>();
        json!({"kind":a.kind,"state":a.state,"reviewed":a.reviewed,"content":content,"content_truncated":content.len()<a.content.len(),"generated":a.is_generated_output()})
    }).collect::<Vec<_>>();
    let edit=m.edit.as_ref().map(|e|json!({"operation":e.operation,"crop":e.crop,"training":e.training,"dimensions":e.dimensions,"verified_original":e.source_archive.as_ref().is_some_and(|a|a.downloaded_original)}));
    Ok(
        json!({"id":m.id,"sha256":m.sha256,"dimensions":m.dimensions(),"original_dimensions":[m.post.image_width,m.post.image_height],"keep":m.keep,"artists":m.post.tag_string_artist,"characters":m.post.tag_string_character,"general_tags":m.post.tag_string_general.chars().take(1000).collect::<String>(),"current_edit":edit,"text":artifacts,"agent_review":review_summary(repo,m)?}),
    )
}
fn review_summary(repo: &SqliteCatalog, m: &StoredMedia) -> Result<Value, IngestError> {
    Ok(repo.agent_review_value(m.id)?.map(|r|json!({"decision":r["decision"],"current":r["image_sha256"]==m.sha256,"stage":r["report"]["stage"],"crop_candidate":r["report"]["crop_candidate"],"requires_final_assessment":r["report"]["stage"]=="preliminary","aesthetic":r["report"]["aesthetic"],"training_suitability":r["report"]["training_suitability"],"reason":r["report"]["reason"].as_str().unwrap_or("").chars().take(160).collect::<String>()})).unwrap_or(Value::Null))
}
fn page(mut rows: Vec<Value>, field: &str) -> ServiceReply {
    while rows.len() > 1 && serde_json::to_vec(&rows).is_ok_and(|b| b.len() > 7800) {
        rows.pop();
    }
    let next = rows.last().and_then(|r| r["id"].as_i64());
    let action = if field == "collections" {
        json!({"tool":"bdm_list_media","hint":"Use the selected collection tag; no need to list collections again."})
    } else if rows.is_empty() {
        json!({"action":"scope_complete","hint":"No more images after this cursor. Report actual results."})
    } else {
        json!({"tool":"bdm_view_image","hint":"View each image needing requested work. Skip already complete annotation in continuation mode; page using next_after_id when these IDs are done."})
    };
    result(json!({(field):rows,"next_after_id":next,"next":action}))
}
fn picture(image: DynamicImage, mut data: Value) -> Result<ServiceReply, IngestError> {
    let native = image.dimensions();
    let display = image.thumbnail(1024, 1024);
    data["native_view_dimensions"] = json!(native);
    data["display_dimensions"] = json!(display.dimensions());
    let mut bytes = std::io::Cursor::new(Vec::new());
    storage(display.write_to(&mut bytes, ImageFormat::Png))?;
    Ok(ServiceReply {
        metadata: data,
        image_png: Some(bytes.into_inner()),
        is_error: false,
    })
}
fn rect(bbox: [u32; 4], dims: (u32, u32)) -> Result<CropRect, IngestError> {
    let [x1, y1, x2, y2] = bbox;
    if x1 >= x2 || y1 >= y2 || x2 > 1000 || y2 > 1000 {
        return Err(guidance_error(
            "invalid_bbox",
            "Use integer [left,top,right,bottom] with 0<=left<right<=1000 and 0<=top<bottom<=1000.",
            json!({"action":"correct_request","fields":["bbox_1000_xyxy"]}),
        ));
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
) -> Result<ServiceReply, IngestError> {
    let needle = args.collection.unwrap_or_default().to_lowercase();
    let mut query=storage(repo.connection.prepare("SELECT c.id,c.tag,c.category,c.provider,(SELECT count(*) FROM collection_media cm WHERE cm.collection_id=c.id) FROM collections c WHERE c.id>?1 AND instr(lower(c.tag),?2)>0 ORDER BY c.id LIMIT ?3"))?;
    let rows=storage(query.query_map(params![args.after_id,needle,args.limit.unwrap_or(32).clamp(1,64)],|r| {
        let category:i64=r.get(2)?;
        Ok(json!({"id":r.get::<_,i64>(0)?,"tag":r.get::<_,String>(1)?,"kind":match category{0=>"query",1=>"artist",4=>"character",_=>"unknown"},"provider":r.get::<_,String>(3)?,"images":r.get::<_,i64>(4)?}))
    }).and_then(|rows|rows.collect::<rusqlite::Result<Vec<_>>>()))?;
    Ok(page(rows, "collections"))
}
pub(super) fn list(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: ListArgs,
) -> Result<ServiceReply, IngestError> {
    let limit = args.limit.unwrap_or(12).clamp(1, 24);
    let ids: Vec<i64> = if let Some(tag) = args.collection {
        let mut q=storage(repo.connection.prepare("SELECT DISTINCT cm.media_id FROM collections c JOIN collection_media cm ON cm.collection_id=c.id WHERE c.tag=?1 AND cm.media_id>?2 ORDER BY cm.media_id LIMIT ?3"))?;
        storage(
            q.query_map(params![tag, args.after_id, limit], |r| r.get(0))
                .and_then(|r| r.collect()),
        )?
    } else {
        let mut q = storage(
            repo.connection
                .prepare("SELECT id FROM media WHERE id>?1 ORDER BY id LIMIT ?2"),
        )?;
        storage(
            q.query_map(params![args.after_id, limit], |r| r.get(0))
                .and_then(|r| r.collect()),
        )?
    };
    let mut rows = Vec::new();
    for id in ids {
        let Some(m) = repo.media_by_id(id)? else {
            continue;
        };
        let existing = artifacts(repo, id)?;
        rows.push(json!({"id":m.id,"sha256":m.sha256,"dimensions":m.dimensions(),"keep":m.keep,"artist":m.post.tag_string_artist,"in_configured_library":checked_path(&m.path,settings).is_ok(),"caption_complete":existing.iter().any(|a|a.kind=="caption"&&a.state=="complete"&&!a.content.is_empty()),"tags_generated":existing.iter().any(|a|a.kind=="tags"&&a.is_generated_output()),"tags_complete":existing.iter().any(|a|a.kind=="tags"&&a.state=="complete"&&a.is_generated_output()),"agent_review":review_summary(repo,&m)?}));
    }
    Ok(page(rows, "images"))
}

pub(super) fn view(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: ViewArgs,
) -> Result<ServiceReply, IngestError> {
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
    if !args.original && args.bbox_1000_xyxy.is_none() {
        data["bucket_recommendation"] = json!(bdm_domain::training::best_bucket(
            m.dimensions().0,
            m.dimensions().1,
            "original"
        )
        .ok());
        let rules = tag_rules(repo, &m)?;
        let details = annotation_details(repo, m.id)?;
        let mut vocabulary = tag_vocabulary(&m, &rules);
        vocabulary["required_tags"] = json!(required_visual_tags(
            &m,
            &rules,
            &artifacts(repo, m.id)?,
            &details
        ));
        vocabulary["retention_rule"] = json!("Submit the full visual tag list, including required_tags. Missing terms block the entire save; overwrite/reason cannot authorize deletion. Do not copy a term contradicted by the current pixels: defer that conflict for human correction. Edited images do not inherit stale source-image requirements.");
        data["tag_vocabulary"] = vocabulary;
        data["annotation_details"] = storage(serde_json::to_value(details))?;
        data["capabilities"] = json!([
            "rich_text_regions",
            "visual_regions",
            "review_reasons",
            "dataset_jsonl"
        ]);
    }
    data["next"] = if args.original {
        json!({"tool":"bdm_view_image","args":{"media_id":m.id,"original":false},"hint":"Original inspection does not authorize annotation of an unseen current image. Restore only when expanding a crop is requested."})
    } else {
        json!({"action":"decide_requested_step","media_id":m.id,"expected_sha256":m.sha256,
            "hint":"Annotation: save caption/tags together using tag_vocabulary. Crop if requested: preview apply=false, inspect, then apply=true. Tiny text only: request a detail tile. Otherwise assess or continue; do not repeat the full view for the same hash."})
    };
    picture(image, data)
}
pub(super) fn crop(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: CropArgs,
) -> Result<ServiceReply, IngestError> {
    provenance(&args.reason, &args.model)?;
    let m = media(repo, settings, args.media_id, Some(&args.expected_sha256))?;
    seen(settings, &m.sha256)?;
    let mut selection = rect(args.bbox_1000_xyxy, m.dimensions())?;
    let ratio=args.ratio.as_deref().filter(|ratio|bdm_domain::training::RATIOS.iter().any(|r|r.0==*ratio))
        .ok_or_else(||guidance_error("crop_ratio_required","Choose one of the existing 17 preset ratios after viewing the image; bbox is a visual proposal, not automatic whole-image alignment.",json!({"tool":"bdm_crop","action":"choose_ratio_and_propose_crop"})))?;
    // Agent proposals use the selected preset and actual Kohya geometry. Native
    // pixels removed by this adjustment are visible in the preview before apply.
    let bucket = {
        let choice = bdm_domain::training::best_bucket(selection.width, selection.height, ratio)
            .map_err(|message| {
                guidance_error(
                    "bucket_unavailable",
                    message,
                    json!({"action":"adjust_crop"}),
                )
            })?;
        selection.x += choice.native_crop[0];
        selection.y += choice.native_crop[1];
        selection.width = choice.native_crop[2];
        selection.height = choice.native_crop[3];
        Some(choice)
    };
    let key = (m.id, m.sha256.clone(), args.bbox_1000_xyxy);
    let receipt = settings
        .previews
        .lock()
        .map_err(|_| IngestError::Storage)?
        .get(&key)
        .cloned();
    if args.apply {
        writes(settings)?;
        if receipt
            .as_ref()
            .is_none_or(|(preview_ratio, _)| preview_ratio != ratio)
        {
            return Err(guidance_error(
                "crop_preview_required",
                "Inspect a preview of this exact crop before applying.",
                json!({"tool":"bdm_crop","args":{"media_id":m.id,"expected_sha256":m.sha256,"bbox_1000_xyxy":args.bbox_1000_xyxy,"ratio":ratio,"apply":false},"hint":"Include reason and model."}),
            ));
        }
    }
    let preview = editing::native_crop_preview(&m, selection, &AtomicBool::new(false))?;
    let preview_sha = editing::hash(&preview.image)?;
    if args.apply {
        if receipt.as_ref().is_none_or(|(_, sha)| *sha != preview_sha)
            || args
                .preview_sha256
                .as_ref()
                .is_some_and(|sha| *sha != preview_sha)
        {
            return Err(guidance_error(
                "crop_preview_required",
                "Crop pixels or chosen ratio differ from the inspected proposal; preview again.",
                json!({"tool":"bdm_crop","action":"preview_exact_ratio_and_bbox"}),
            ));
        }
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
            json!({"saved":true,"media":info(repo,&saved)?,"bucket_plan":bucket,"next":{"tool":"bdm_view_image","args":{"media_id":m.id,"original":false},"hint":"Inspect the saved pixels; use the new hash before annotation."}}),
        ))
    } else {
        settings
            .previews
            .lock()
            .map_err(|_| IngestError::Storage)?
            .insert(key, (ratio.to_owned(), preview_sha.clone()));
        picture(
            storage(image::open(&preview.image))?,
            json!({"saved":false,"media_id":m.id,"expected_sha256":m.sha256,"selection":selection,"training":preview.training,"ratio":ratio,"bucket_plan":bucket,"preview_sha256":preview_sha,"bbox_1000_xyxy":args.bbox_1000_xyxy,
                "proposal":{"ratio":ratio,"reason":args.reason,"model":args.model,"source_bbox_1000_xyxy":args.bbox_1000_xyxy,"native_selection":selection,"ratio_tolerance":0.02},
                "next":{"tool":"bdm_crop","args":{"media_id":m.id,"expected_sha256":m.sha256,"bbox_1000_xyxy":args.bbox_1000_xyxy,"ratio":ratio,"preview_sha256":preview_sha,"apply":true},"hint":"Apply only if this preview improves composition; retain reason and model. Otherwise revise preview or keep unchanged."}}),
        )
    }
}
pub(super) fn restore(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: RestoreArgs,
) -> Result<ServiceReply, IngestError> {
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
        return Err(guidance_error(
            "original_not_viewed",
            "Inspect the complete original before replacing the current edit.",
            json!({"tool":"bdm_view_image","args":{"media_id":m.id,"original":true},"hint":"Restore only to expand a prior crop, then inspect current again."}),
        ));
    }
    let saved = repo.restore_downloaded_original(m.id)?;
    repo.audit(
        "agent_restore",
        &json!({"id":m.id,"reason":args.reason,"model":args.model}).to_string(),
    )?;
    let mut response = info(repo, &saved)?;
    response["next"] = json!({"tool":"bdm_view_image","args":{"media_id":m.id,"original":false},"hint":"Restored pixels require fresh inspection before crop or annotation."});
    Ok(result(response))
}

pub(super) fn annotate(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: AnnotationArgs,
) -> Result<ServiceReply, IngestError> {
    writes(settings)?;
    let source = provenance(&args.reason, &args.model)?;
    let lock = repo.media_lock.clone();
    let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
    let m = media(repo, settings, args.media_id, Some(&args.expected_sha256))?;
    seen(settings, &m.sha256)?;
    if args.require_final_assessment {
        let review = repo.agent_review_value(m.id)?;
        let eligible = review.as_ref().is_some_and(|r| {
            r["image_sha256"] == m.sha256
                && r["decision"] == "keep"
                && r["report"]["stage"].as_str().unwrap_or("final") == "final"
        });
        if !eligible {
            return Err(guidance_error("final_assessment_required", "Full curation must assess and pass the actual final image before writing its training annotation.",
                json!({"tool":"bdm_assess","args":{"media_id":m.id,"expected_sha256":m.sha256,"stage":"final"},"hint":"Resolve any pending crop first. Standalone annotation may omit require_final_assessment."})));
        }
    }
    let regions_only = args.caption.is_none()
        && args.tags.is_none()
        && (args.text_regions.is_some()
            || args.visual_regions.is_some()
            || args.special_elements.is_some());
    if args.caption.is_none()
        && args.tags.is_none()
        && args.review_reasons.is_none()
        && !regions_only
    {
        return Err(guidance_error(
            "empty_annotation",
            "Supply caption and/or tags, or review_reasons for a review-only decision.",
            json!({"tool":"bdm_save_annotation","hint":"caption is visual prose; tags is an array of generic terms."}),
        ));
    }
    let rules = tag_rules(repo, &m)?;
    let normalized = bdm_domain::tags::normalize(
        &m.post.tag_string,
        &m.post.tag_string_artist,
        &m.post.tag_string_character,
        &rules,
    );
    let existing = artifacts(repo, m.id)?;
    let mut saved = Vec::new();
    let mut preserved = Vec::new();
    let mut readback = Vec::new();
    let mut outputs = Vec::new();
    let mut rejected = Vec::new();
    let mut rejection_details = Vec::new();
    let mut accepted = Vec::new();
    // Decide preservation before validating unused replacement text. A protected
    // caption must not block a tags-only continuation (or vice versa).
    for (kind, provided) in [
        ("caption", args.caption.is_some()),
        ("tags", args.tags.is_some()),
    ] {
        if !provided {
            continue;
        }
        let path = m
            .path
            .with_extension(if kind == "tags" { "tags.txt" } else { "txt" });
        let protected = existing
            .iter()
            .any(|a| a.kind == kind && (a.is_generated_output() || a.reviewed));
        let manual = match fs::read(&path) {
            Ok(bytes) => {
                !bytes.iter().all(u8::is_ascii_whitespace)
                    && !existing
                        .iter()
                        .any(|a| a.kind == kind && a.content.as_bytes() == bytes)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(IngestError::StorageDetails(error.to_string())),
        };
        if !args.overwrite && (protected || manual) {
            preserved.push(kind);
        }
    }
    if let Some(caption) = &args.caption {
        if !preserved.contains(&"caption") {
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
                return Err(guidance_error("invalid_caption", "Use nonempty visual prose (<=8192 bytes), without artist/character/copyright identity prefixes or control characters. Put exact visible signatures in text_regions.",
                    json!({"tool":"bdm_save_annotation","args":{"media_id":m.id,"expected_sha256":m.sha256},"hint":"Correct caption only; do not prepend identities. Server adds them."})));
            }
            outputs.push((
                "caption",
                ai::format_caption(caption, &normalized, "at", "plain"),
            ));
        }
    }
    if let Some(tags) = &args.tags {
        if !preserved.contains(&"tags") {
            if tags.is_empty() || tags.len() > 512 || tags.iter().any(|t| t.len() > 256) {
                return Err(guidance_error(
                    "invalid_tags",
                    "tags must contain 1..512 strings of at most 256 bytes each.",
                    json!({"tool":"bdm_save_annotation","hint":"Submit an array, not a comma-separated string."}),
                ));
            }
            for tag in tags {
                match ai::identity::check_visual_tag(tag, &rules) {
                    Ok(term) => {
                        if !accepted.contains(&term) {
                            accepted.push(term);
                        }
                    }
                    Err(code) => {
                        rejected.push(tag.clone());
                        rejection_details.push(json!({"tag":tag,"code":code}));
                    }
                }
            }
            if !accepted.is_empty() {
                let required =
                    required_visual_tags(&m, &rules, &existing, &annotation_details(repo, m.id)?);
                let missing: Vec<_> = required
                    .iter()
                    .filter(|tag| !accepted.contains(tag))
                    .collect();
                if !missing.is_empty() {
                    return Err(guidance_error("tag_coverage_incomplete", "Ordinary visual tags cannot be silently dropped. Nothing in this call was saved. Submit the complete visually checked tag list; neither overwrite nor a reason bypasses retention. If an existing term contradicts the pixels, defer for human correction instead of copying it blindly.",
                        json!({"tool":"bdm_save_annotation","args":{"media_id":m.id,"expected_sha256":m.sha256},"missing_tags":missing,"required_tags":required,"accepted_tags":accepted,"tag_rejections":rejection_details,"hint":"Preserve all still-visible detail, not just key tags. Do not pad with unrelated tags. Reuse this image inspection; no new view is needed unless detail is uncertain."})));
                }
                // Merge only verified identities and model-verified visual tags.
                // Source meta/protected tags must never leak back into training TXT.
                outputs.push((
                    "tags",
                    ai::format_caption(&accepted.join(", "), &normalized, "at", "plain"),
                ));
            }
        }
    }
    // Omitted structured fields preserve prior metadata; explicit [] clears it.
    let mut details = annotation_details(repo, m.id)?;
    let previous_details = details.clone();
    let details_requested = args.text_regions.is_some()
        || args.special_elements.is_some()
        || args.visual_regions.is_some();
    if details
        .image_sha256
        .as_ref()
        .is_some_and(|sha| sha != &m.sha256)
        && ((!details.text_regions.is_empty() && args.text_regions.is_none())
            || (!details.visual_regions.is_empty() && args.visual_regions.is_none())
            || (!details.special_elements.is_empty() && args.special_elements.is_none()))
    {
        return Err(guidance_error("stale_annotation_details", "Regions refer to an older crop. Inspect the current image and explicitly replace or clear old region fields.",
            json!({"tool":"bdm_view_image","args":{"media_id":m.id,"original":false},"hint":"Recheck text_regions, visual_regions and special_elements for the new pixels."})));
    }
    if let Some(regions) = args.text_regions {
        if regions.len() > 64 || regions.iter().any(|r| !r.is_valid()) {
            return Err(guidance_error("invalid_text_regions", "Use <=64 regions with valid bbox/kind. Legibility: readable/partial/unreadable/decorative. Unreadable/decorative text is exactly 'unreadable text'; partial contains only recognized characters. Optional confidence is 0..1, language <=64 bytes, appearance <=1024 bytes.",
                json!({"tool":"bdm_save_annotation","hint":"Coordinates refer to the full current image. Fix the region, not the caption."})));
        }
        details.text_regions = regions;
    }
    if let Some(special) = args.special_elements {
        if special.len() > 2
            || special
                .iter()
                .any(|e| !matches!(e.as_str(), "x_ray" | "cross_section"))
            || special.iter().collect::<BTreeSet<_>>().len() != special.len()
        {
            return Err(guidance_error(
                "invalid_special_elements",
                "Only unique x_ray and cross_section are accepted.",
                json!({"tool":"bdm_save_annotation","hint":"Keep protocol enum underscores; [] explicitly clears the field."}),
            ));
        }
        details.special_elements = special;
    }
    if let Some(regions) = args.visual_regions {
        if regions.len() > 64 || regions.iter().any(|r| !r.is_valid()) {
            return Err(guidance_error("invalid_visual_regions", "Use <=64 visual regions, normalized bbox, supported kind, description <=2048 bytes, relation <=512 bytes, confidence null or 0..1.",
                json!({"tool":"bdm_save_annotation","hint":"Kinds: subject/background/sticker/overlay/embedded_image/detail_inset/x_ray/cross_section. Do not confuse inset cutaways with superimposed x-ray views."})));
        }
        details.visual_regions = regions;
    }
    if args.review_reasons.as_ref().is_some_and(|reasons| {
        reasons.len() > 16
            || reasons
                .iter()
                .any(|r| r.trim().is_empty() || r.len() > 512 || r.chars().any(char::is_control))
    }) {
        return Err(guidance_error(
            "invalid_review_reasons",
            "Use <=16 nonempty review reasons, each <=512 bytes without control characters.",
            json!({"action":"correct_request","fields":["review_reasons"]}),
        ));
    }
    if args.require_final_assessment {
        let review = repo.agent_review_value(m.id)?.ok_or(IngestError::Storage)?;
        let mut required: Vec<RetainedElement> =
            serde_json::from_value(review["report"]["retained_elements"].clone())
                .unwrap_or_default();
        if review["report"]["watermark_present"] == true
            && !required.iter().any(|e| {
                matches!(
                    e,
                    RetainedElement::Watermark | RetainedElement::Logo | RetainedElement::Signature
                )
            })
        {
            required.push(RetainedElement::Watermark);
        }
        let missing: Vec<_> = required
            .into_iter()
            .filter(|element| !match element {
                RetainedElement::Text => details.text_regions.iter().any(|r| r.kind != "watermark"),
                RetainedElement::Watermark => {
                    details.text_regions.iter().any(|r| r.kind == "watermark")
                        || details.visual_regions.iter().any(|r| r.kind == "watermark")
                }
                RetainedElement::Signature => {
                    details.text_regions.iter().any(|r| r.kind == "watermark")
                        || details.visual_regions.iter().any(|r| r.kind == "signature")
                }
                RetainedElement::Logo => details.visual_regions.iter().any(|r| r.kind == "logo"),
                RetainedElement::Sticker => {
                    details.visual_regions.iter().any(|r| r.kind == "sticker")
                }
                RetainedElement::Overlay => {
                    details.visual_regions.iter().any(|r| r.kind == "overlay")
                        || details.text_regions.iter().any(|r| r.kind == "overlay")
                }
            })
            .collect();
        if !missing.is_empty() {
            return Err(guidance_error("retained_elements_bbox_required","Final inspection reported retained text/watermarks/stickers. Supply matching BBoxes on the final current image before saving; preserve composition instead of forcing removal.",json!({"tool":"bdm_save_annotation","missing":missing,"hint":"OCR in text_regions; non-text decorations in visual_regions. Omitted fields can reuse verified current-image regions."})));
        }
    }
    for (kind, content) in outputs {
        repo.write_artifact_locked(&m, kind, &content, &source, "complete", true)?;
        saved.push(kind);
    }
    if (!saved.is_empty() || regions_only) && details_requested {
        details.schema_version = "agent-regions-v1".into();
        details.image_sha256 = Some(m.sha256.clone());
        details.user_hint = Some(args.reason.clone());
    } else {
        // Do not report unsaved proposed regions as if they were persisted.
        details = previous_details;
    }
    if let Some(reasons) = &args.review_reasons {
        details.review_reasons = reasons.clone();
        if !reasons.is_empty() {
            state::save_review(
                repo,
                m.id,
                &m.sha256,
                "defer",
                &json!({"reason":reasons.join("; "),"model":args.model,"review_reasons":reasons}),
                None,
            )?;
            storage(
                repo.connection
                    .execute("UPDATE media SET keep=0 WHERE id=?1", [m.id]),
            )?;
        }
    }
    let current = artifacts(repo, m.id)?;
    let tags_saved = saved.contains(&"tags");
    if tags_saved {
        let tags = current
            .iter()
            .find(|a| a.kind == "tags")
            .ok_or(IngestError::Storage)?;
        details.tags_image_sha256 = Some(m.sha256.clone());
        details.tags_content_sha256 = Some(format!(
            "{:x}",
            sha2::Sha256::digest(tags.content.as_bytes())
        ));
    }
    let details_saved = ((!saved.is_empty() || regions_only) && details_requested)
        || args.review_reasons.is_some()
        || tags_saved;
    if details_saved {
        repo.save_annotation_details(m.id, &details)?;
    }
    let mut verified = true;
    for a in current.iter().filter(|a| saved.contains(&a.kind.as_str())) {
        let path = m
            .path
            .with_extension(if a.kind == "tags" { "tags.txt" } else { "txt" });
        let matches = storage(fs::read(path))? == a.content.as_bytes();
        verified &= matches;
        let content = a.content.chars().take(1600).collect::<String>();
        readback.push(json!({"kind":a.kind,"content":content,"content_truncated":content.len()<a.content.len(),
            "sha256":format!("{:x}",sha2::Sha256::digest(a.content.as_bytes())),"sidecar_matches":matches}));
    }
    let status = if !verified {
        "verification_failed"
    } else if !rejected.is_empty() {
        if saved.is_empty() {
            "needs_correction"
        } else {
            "partial"
        }
    } else if saved.is_empty() && args.review_reasons.is_some() {
        "review_saved"
    } else if saved.is_empty() {
        "preserved"
    } else {
        "saved"
    };
    let next = if !verified {
        json!({"action":"stop_and_report","hint":"Saved file differs from database. Do not repeat model calls."})
    } else if !details.review_reasons.is_empty() {
        json!({"action":"human_review_queued","hint":"Do not automatically approve or export this image. Continue other requested IDs; clearing reasons alone does not approve it."})
    } else if !rejected.is_empty() {
        json!({"tool":"bdm_save_annotation","args":{"media_id":m.id,"expected_sha256":m.sha256},
            "hint":"Correct rejected tags only if needed; omit saved caption. Unknown terms may stay in prose. Accepted tags are already saved: overwrite only with authorization. Do not retry unchanged terms."})
    } else {
        json!({"action":"continue_requested_workflow","hint":"Check readback semantics, then assess/annotate the next inspected image as requested. No extra list/view call is needed just to confirm this save."})
    };
    repo.audit("agent_annotation",&json!({"id":m.id,"sha256":m.sha256,"saved":saved,"preserved":preserved,"rejected_tags":rejection_details,"status":status,"model":args.model,"reason":args.reason,
        "annotation_details":if details_saved {Some(&details)}else{None}}).to_string())?;
    let mut response = json!({"media_id":m.id,"expected_sha256":m.sha256,"status":status,"saved":saved,"preserved":preserved,
        "accepted_tags":accepted,"rejected_tags":rejected,"tag_rejections":rejection_details,"readback":readback,
        "verification":{"sidecars_match":verified,"semantic_approval":false},
        "annotation_details":{"status":if details_saved {"saved"} else {"preserved"},"image_sha256":details.image_sha256,"text_region_count":details.text_regions.len(),"visual_region_count":details.visual_regions.len(),"special_elements":details.special_elements,"review_reasons":details.review_reasons},"next":next});
    if !rejected.is_empty() {
        response["tag_vocabulary"] = tag_vocabulary(&m, &rules);
    }
    Ok(result(response))
}

pub(super) fn assess(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: AssessArgs,
) -> Result<ServiceReply, IngestError> {
    writes(settings)?;
    provenance(&args.reason, &args.model)?;
    if args.retained_elements.as_ref().is_some_and(|items| {
        items.len() > 6 || items.iter().collect::<BTreeSet<_>>().len() != items.len()
    }) {
        return Err(guidance_error("invalid_retained_elements","Use each retained element kind at most once: text/watermark/sticker/overlay/logo/signature.",json!({"tool":"bdm_assess","action":"correct_final_inventory"})));
    }
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
    let preliminary = matches!(args.stage, AssessmentStage::Preliminary);
    if !preliminary && args.crop_candidate || preliminary && args.crop_outcome.is_some() {
        return Err(guidance_error("invalid_assessment_stage", "crop_candidate belongs to preliminary assessment; crop_outcome belongs to final assessment.",
            json!({"action":"correct_request","fields":["stage","crop_candidate","crop_outcome"]})));
    }
    let previous = repo.agent_review_value(m.id)?;
    let pending = previous
        .as_ref()
        .filter(|r| r["report"]["stage"] == "preliminary" && r["report"]["crop_candidate"] == true);
    if preliminary && pending.is_some_and(|r| !args.crop_candidate || r["image_sha256"] != m.sha256)
    {
        return Err(guidance_error(
            "crop_first_required",
            "Resolve the existing crop candidate before starting another preliminary assessment.",
            json!({"tool":"bdm_assess","args":{"media_id":m.id,"expected_sha256":m.sha256,"stage":"final"},"hint":"Use verified applied or not_improved crop_outcome; do not reset the candidate."}),
        ));
    }
    if !preliminary {
        if let Some(pending) = pending {
            let before = pending["image_sha256"].as_str().unwrap_or_default();
            let valid = match args.crop_outcome {
                Some(CropOutcome::Applied) => {
                    m.sha256 != before
                        && m.edit
                            .as_ref()
                            .is_some_and(|e| e.operation == "crop" && e.parent_sha256 == before)
                }
                Some(CropOutcome::NotImproved) => {
                    m.sha256 == before
                        && settings
                            .previews
                            .lock()
                            .map_err(|_| IngestError::Storage)?
                            .iter()
                            .any(|((id, sha, _), _)| *id == m.id && sha == &m.sha256)
                }
                None => false,
            };
            if !valid {
                return Err(guidance_error("crop_first_required", "This image was marked improvable by crop. Preview a safe crop, then apply/reinspect/reassess it, or report not_improved after inspecting a preview without changing the original.",
                    json!({"tool":"bdm_crop","args":{"media_id":m.id,"expected_sha256":m.sha256,"apply":false},"hint":"Final crop_outcome needs verified applied or not_improved evidence. Do not queue an early rejection."})));
            }
        } else if args.crop_outcome.is_some() {
            return Err(guidance_error(
                "no_pending_crop",
                "No pending preliminary crop assessment exists for this image.",
                json!({"action":"correct_request","hint":"Omit crop_outcome for ordinary final assessment."}),
            ));
        }
    }
    if (preliminary || matches!(args.decision, Decision::Keep))
        && !annotation_details(repo, m.id)?.review_reasons.is_empty()
    {
        return Err(guidance_error(
            "human_review_required",
            "Resolve manual-review concerns before automatic crop assessment or approval.",
            json!({"action":"leave_for_human_review","hint":"Resolve the recorded concern before explicitly clearing review_reasons and approving."}),
        ));
    }
    let decision = match (preliminary, args.decision) {
        (true, _) => "defer",
        (false, Decision::Keep) => "keep",
        (false, Decision::Reject) => "reject",
        (false, Decision::Defer) => "defer",
    };
    let mut report = storage(serde_json::to_value(&args))?;
    report["proposed_decision"] = json!(args.decision);
    report["decision"] = json!(decision);
    let transaction = storage(repo.connection.unchecked_transaction())?;
    state::save_review(repo, m.id, &m.sha256, decision, &report, None)?;
    if !preliminary {
        storage(repo.connection.execute("INSERT INTO media_quality(media_id,aesthetic,watermark_free,provenance) VALUES(?1,?2,?3,'{}') ON CONFLICT(media_id) DO UPDATE SET aesthetic=excluded.aesthetic,watermark_free=excluded.watermark_free",params![m.id,args.aesthetic,args.watermark_present.map(|v|!v)]))?;
    }
    storage(repo.connection.execute(
        "UPDATE media SET keep=?2 WHERE id=?1",
        params![m.id, decision == "keep"],
    ))?;
    repo.audit("agent_assessment", &report.to_string())?;
    storage(transaction.commit())?;
    let next = if preliminary && args.crop_candidate {
        json!({"tool":"bdm_crop","args":{"media_id":m.id,"expected_sha256":m.sha256,"apply":false},"hint":"Choose a composition-preserving crop using existing ratios. Preview then apply, inspect the saved current image, and call final assessment with crop_outcome=applied. If the preview harms expression, leave original and reassess with not_improved."})
    } else if preliminary {
        json!({"tool":"bdm_assess","args":{"media_id":m.id,"expected_sha256":m.sha256,"stage":"final"},"hint":"Preliminary scores are not final approval/rejection."})
    } else if decision == "keep" {
        json!({"tool":"bdm_save_annotation","args":{"media_id":m.id,"expected_sha256":m.sha256,"require_final_assessment":true},"hint":"If annotation was requested, describe the actual final pixels; omit watermarks removed by crop. Otherwise continue the requested workflow."})
    } else {
        json!({"action":"continue_next_image","hint":"Reject is pending human deletion review; defer awaits review. Do not generate full-curation training TXT for this image."})
    };
    Ok(result(
        json!({"id":m.id,"stage":args.stage,"crop_candidate":args.crop_candidate,"decision":decision,"queued_for_deletion":decision=="reject","files_deleted":0,"next":next}),
    ))
}

pub(super) fn scan(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: ScanArgs,
) -> Result<ServiceReply, IngestError> {
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
    // Analysis runs outside the media mutation lock. visual::run revalidates
    // and commits under a short lock for each analyzed image.
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
        json!({"state":job.state,"error":job.error,"candidate_count":count,"candidates":candidates.into_iter().take(24).collect::<Vec<_>>(),"more_pending":count>24,"files_deleted":0,
            "next":{"action":"inspect_pairs_or_report_scan_error","tool":"bdm_view_image","hint":"On analysis failure report it; do not claim dedup complete. Otherwise view both candidates, resolve, then rescan only if more pending pairs remain."}}),
    ))
}

pub(super) fn pair(
    repo: &mut SqliteCatalog,
    settings: &Settings,
    args: PairArgs,
) -> Result<ServiceReply, IngestError> {
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
    Ok(result(
        json!({"decision":report,"files_deleted":0,"next":{"action":"continue_requested_workflow","hint":"Resolve the next returned pair; rescan if more_pending was true. Final deletion stays in the UI review queue."}}),
    ))
}
