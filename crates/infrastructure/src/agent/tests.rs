use super::*;
use bdm_ports::{CollectionRepository, IngestRepository};
use sha2::Digest;
use std::{collections::BTreeSet, fs, sync::atomic::AtomicBool};

fn fixture() -> (
    crate::workflow_tests::Fixture,
    Settings,
    bdm_domain::ingest::StoredMedia,
) {
    let mut f = crate::workflow_tests::fixture();
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            f.collection,
            800,
            &AtomicBool::new(false),
            &f.provider,
        )
        .unwrap();
    let m = f.repo.list_media().unwrap().remove(0);
    let settings = Settings {
        project_root: f.directory.path().to_owned(),
        library_root: f.directory.path().join("library").canonicalize().unwrap(),
        writes: true,
        permit: None,
        seen: Mutex::default(),
        previews: Mutex::default(),
    };
    (f, settings, m)
}

fn service(f: &crate::workflow_tests::Fixture, writes: bool) -> AgentService {
    AgentService::open(
        &f.directory.path().join("catalog.sqlite3"),
        f.directory.path(),
        &f.directory.path().join("library"),
        writes,
    )
    .unwrap()
}
fn ok(service: &AgentService, op: &str, input: serde_json::Value) -> serde_json::Value {
    let reply = service.execute(op, input);
    assert!(!reply.is_error, "{op}: {}", reply.metadata);
    reply.metadata
}
fn leased_input(
    item: &serde_json::Value,
    request: &str,
    payload: serde_json::Value,
) -> serde_json::Value {
    let mut input = item.clone();
    input["request_id"] = json!(request);
    input["payload"] = payload;
    input
}
fn start_work(
    f: &crate::workflow_tests::Fixture,
    id: i64,
    mode: &str,
) -> (AgentService, serde_json::Value, i64) {
    let s = service(f, true);
    let submitted = ok(
        &s,
        "job_submit",
        json!({"request_id":"job-1","media_ids":[id],"mode":mode,"outputs":["caption"]}),
    );
    let job = submitted["job_id"].as_i64().unwrap();
    ok(&s, "jobs_tick", json!({}));
    let item = ok(&s, "work_claim", json!({"owner":"test","job_id":job}))["items"][0].clone();
    assert!(item["work_id"].as_i64().is_some());
    (s, item, job)
}

#[test]
fn durable_work_survives_new_clients_and_replays_once_without_model_or_gui() {
    let (f, _, m) = fixture();
    let (s, item, job) = start_work(&f, m.id, "annotate");
    let input = leased_input(
        &item,
        "save-1",
        json!({"caption":"A blue square with flat color.","reason":"Observed current pixels","model":"test"}),
    );
    assert!(s.execute("work_annotate", input.clone()).is_error);
    ok(&s, "work_view", leased_input(&item, "view", json!({})));
    drop(s);
    let s = service(&f, true);
    let saved = ok(&s, "work_annotate", input.clone());
    assert_eq!(saved["next_stage"], "completed");
    let replay = ok(&service(&f, true), "work_annotate", input.clone());
    assert_eq!(saved, replay);
    let mut changed = input.clone();
    changed["payload"]["caption"] = json!("Another caption.");
    assert_eq!(
        s.execute("work_annotate", changed).metadata["code"],
        "idempotency_conflict"
    );
    let mut forged = input;
    forged["lease_token"] = json!("not-the-lease");
    assert_eq!(
        s.execute("work_annotate", forged).metadata["code"],
        "lease_lost"
    );
    assert_eq!(
        ok(&s, "job_status", json!({"job_id":job}))["state"],
        "completed"
    );
    let logs: i64 = f
        .repo
        .connection
        .query_row(
            "SELECT count(*) FROM audit_log WHERE action='agent_annotation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(logs, 1);
    assert!(
        f.repo
            .list_workflows()
            .unwrap()
            .iter()
            .find(|j| j.id == job)
            .unwrap()
            .dismissed
    );
}

#[test]
fn pause_fences_old_lease_and_cancel_cannot_be_revived() {
    let (f, _, m) = fixture();
    let (s, item, job) = start_work(&f, m.id, "annotate");
    ok(&s, "job_control", json!({"job_id":job,"action":"pause"}));
    assert_eq!(
        s.execute("work_view", leased_input(&item, "view", json!({})))
            .metadata["code"],
        "lease_lost"
    );
    ok(&s, "job_control", json!({"job_id":job,"action":"resume"}));
    let new = ok(&s, "work_claim", json!({"owner":"new","job_id":job}))["items"][0].clone();
    assert!(new["generation"].as_i64() > item["generation"].as_i64());
    ok(&s, "job_control", json!({"job_id":job,"action":"cancel"}));
    assert!(
        s.execute("job_control", json!({"job_id":job,"action":"resume"}))
            .is_error
    );
    assert_eq!(
        ok(&s, "job_status", json!({"job_id":job}))["state"],
        "cancelled"
    );
}

#[test]
fn two_clients_cannot_claim_same_image_and_expiry_reissues_fenced_generation() {
    let (f, _, m) = fixture();
    let (s, item, job) = start_work(&f, m.id, "annotate");
    assert!(
        ok(&service(&f, true), "work_claim", json!({"owner":"second"}))["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    f.repo
        .connection
        .execute(
            "UPDATE agent_work SET expires=1 WHERE id=?1",
            [item["work_id"].as_i64().unwrap()],
        )
        .unwrap();
    let next = ok(&s, "work_claim", json!({"owner":"second","job_id":job}))["items"][0].clone();
    assert!(next["generation"].as_i64() > item["generation"].as_i64());
    assert_eq!(
        s.execute("work_view", leased_input(&item, "v", json!({})))
            .metadata["code"],
        "lease_lost"
    );
}

#[test]
fn a_new_owner_must_inspect_again_even_if_prior_owner_finished_while_pausing() {
    let (f, _, m) = fixture();
    let (s, item, job) = start_work(&f, m.id, "annotate");
    ok(&s, "work_view", leased_input(&item, "v", json!({})));
    // Models the idle handoff after an in-flight write finished under pause:
    // its token is cleared but its old view receipt still exists in storage.
    f.repo
        .connection
        .execute(
            "UPDATE agent_work SET lease=NULL,owner=NULL WHERE id=?1",
            [item["work_id"].as_i64().unwrap()],
        )
        .unwrap();
    let next = ok(&s, "work_claim", json!({"owner":"new-owner","job_id":job}))["items"][0].clone();
    let response = s.execute(
        "work_annotate",
        leased_input(
            &next,
            "save",
            json!({"caption":"Uninspected suggestion.","reason":"Test","model":"test"}),
        ),
    );
    assert_eq!(response.metadata["code"], "inspection_required");
    assert_eq!(
        s.execute("work_release", item).metadata["code"],
        "lease_lost"
    );
    ok(&s, "work_view", leased_input(&next, "v", json!({})));
    ok(
        &s,
        "work_annotate",
        leased_input(
            &next,
            "save",
            json!({"caption":"A blue square.","reason":"Inspected current image","model":"test"}),
        ),
    );
}

#[test]
fn task_plan_prevents_skipping_curation_or_overwriting_unselected_outputs() {
    let (f, _, m) = fixture();
    let (s, item, _) = start_work(&f, m.id, "curate");
    ok(&s, "work_view", leased_input(&item, "v", json!({})));
    let annotate = leased_input(
        &item,
        "a",
        json!({"caption":"Blue.","reason":"Test","model":"test","require_final_assessment":false}),
    );
    assert_eq!(
        s.execute("work_annotate", annotate).metadata["code"],
        "wrong_stage"
    );
    let mut payload = serde_json::to_value(assessment(&m, Decision::Keep)).unwrap();
    payload.as_object_mut().unwrap().remove("media_id");
    payload.as_object_mut().unwrap().remove("expected_sha256");
    ok(&s, "work_assess", leased_input(&item, "assess", payload));
    let bad = leased_input(
        &item,
        "bad",
        json!({"tags":["1girl"],"reason":"Test","model":"test"}),
    );
    assert_eq!(
        s.execute("work_annotate", bad).metadata["code"],
        "output_not_requested"
    );
    // A validation failure is correctable, not an ambiguous committed write.
    ok(
        &s,
        "work_annotate",
        leased_input(
            &item,
            "good",
            json!({"caption":"A blue square.","reason":"Test","model":"test"}),
        ),
    );
}

#[test]
fn regions_can_be_saved_without_rewriting_text_and_readonly_rejects_job_submission() {
    let (f, settings, m) = fixture();
    let s = service(&f, true);
    let job = ok(
        &s,
        "job_submit",
        json!({"request_id":"regions","media_ids":[m.id],"mode":"annotate","outputs":["regions"]}),
    )["job_id"]
        .clone();
    ok(&s, "jobs_tick", json!({}));
    let item = ok(&s, "work_claim", json!({"owner":"test","job_id":job}))["items"][0].clone();
    ok(&s, "work_view", leased_input(&item, "v", json!({})));
    let before = fs::read(m.path.with_extension("txt")).ok();
    let result = ok(
        &s,
        "work_annotate",
        leased_input(
            &item,
            "regions",
            json!({"text_regions":[],"visual_regions":[],"special_elements":[],"reason":"No visible lettering or insets","model":"test"}),
        ),
    );
    assert_eq!(result["next_stage"], "completed");
    assert_eq!(before, fs::read(m.path.with_extension("txt")).ok());
    let readonly = service(&f, false);
    assert_eq!(
        readonly.execute("job_submit", json!({})).metadata["code"],
        "read_only"
    );
    assert!(tools::media(&f.repo, &settings, m.id, None).is_ok());
}

#[test]
fn full_curation_crop_has_native_pixels_matching_bucket_and_requires_reinspection() {
    let (f, _, m) = fixture();
    let (s, mut item, job) = start_work(&f, m.id, "curate");
    ok(&s, "work_view", leased_input(&item, "view", json!({})));
    let mut assessment = serde_json::to_value(assessment(&m, Decision::Reject)).unwrap();
    assessment.as_object_mut().unwrap().remove("media_id");
    assessment
        .as_object_mut()
        .unwrap()
        .remove("expected_sha256");
    assessment["crop_candidate"] = json!(true);
    let preliminary = ok(
        &s,
        "work_assess",
        leased_input(&item, "triage", assessment.clone()),
    );
    assert_eq!(preliminary["next_stage"], "crop");
    assert_eq!(preliminary["decision"], "defer");
    let mut crop = json!({"ratio":"1:1","bbox_1000_xyxy":[20,20,980,980],"reason":"Keep subject; trim empty margins","model":"test"});
    let preview = ok(
        &s,
        "work_preview",
        leased_input(&item, "preview", crop.clone()),
    );
    assert_eq!(preview["next"]["operation"], "work_apply");
    let no_echo = s.execute(
        "work_apply",
        leased_input(&item, "missing-preview-proof", crop.clone()),
    );
    assert_eq!(no_echo.metadata["code"], "preview_required");
    crop["preview_sha256"] = preview["preview_sha256"].clone();
    let mut changed_ratio = crop.clone();
    changed_ratio["ratio"] = json!("2:1");
    assert_eq!(
        s.execute(
            "work_apply",
            leased_input(&item, "changed-ratio", changed_ratio)
        )
        .metadata["code"],
        "preview_required"
    );
    drop(s);
    let s = service(&f, true);
    let saved = ok(&s, "work_apply", leased_input(&item, "crop", crop));
    assert_eq!(saved["next"]["operation"], "work_view");
    let current = f.repo.media_by_id(m.id).unwrap().unwrap();
    assert_ne!(current.sha256, m.sha256);
    assert_eq!(current.current_extension(), "png");
    let actual = bdm_domain::training::plan(
        current.dimensions().0,
        current.dimensions().1,
        saved["bucket_plan"]["resolution"].as_u64().unwrap() as u32,
    )
    .unwrap();
    assert_eq!(actual.resized, actual.output);
    assert_eq!(current.dimensions(), (768, 768));
    item["expected_sha256"] = json!(current.sha256);
    assessment["crop_candidate"] = json!(false);
    assessment["crop_outcome"] = json!("applied");
    assessment["decision"] = json!("keep");
    assert_eq!(
        s.execute(
            "work_assess",
            leased_input(&item, "final", assessment.clone())
        )
        .metadata["code"],
        "inspection_required"
    );
    ok(&s, "work_view", leased_input(&item, "new-view", json!({})));
    ok(&s, "work_assess", leased_input(&item, "final", assessment));
    ok(
        &s,
        "work_annotate",
        leased_input(
            &item,
            "caption",
            json!({"caption":"A blue square with uniform color.","reason":"Final current crop inspected","model":"test"}),
        ),
    );
    assert_eq!(
        ok(&s, "job_status", json!({"job_id":job}))["state"],
        "completed"
    );
}

#[test]
fn retained_text_watermark_and_sticker_need_boxes_without_forcing_a_crop() {
    let (f, _, m) = fixture();
    let s = service(&f, true);
    ok(
        &s,
        "job_submit",
        json!({"request_id":"preserve-composition","mode":"curate","media_ids":[m.id],"outputs":["caption","regions"]}),
    );
    ok(&s, "jobs_tick", json!({}));
    let item = ok(&s, "work_claim", json!({"owner":"test"}))["items"][0].clone();
    ok(&s, "work_view", leased_input(&item, "v", json!({})));
    let mut report = serde_json::to_value(assessment(&m, Decision::Keep)).unwrap();
    report.as_object_mut().unwrap().remove("media_id");
    report.as_object_mut().unwrap().remove("expected_sha256");
    report["retained_elements"] = json!(["text", "watermark", "sticker"]);
    report["watermark_present"] = json!(true);
    report["reason"]=json!("Removing these overlays would cut the subject and meaningful setting; preserve the composition and annotate every retained region.");
    let assessed = ok(&s, "work_assess", leased_input(&item, "assess", report));
    assert_eq!(assessed["next_stage"], "annotation");
    let missing = json!({"caption":"A blue field with white lettering and a small decorative sticker.","text_regions":[],"visual_regions":[],"reason":"Test","model":"test"});
    assert_eq!(
        s.execute(
            "work_annotate",
            leased_input(&item, "missing", missing.clone())
        )
        .metadata["code"],
        "retained_elements_bbox_required"
    );
    let mut annotated = missing;
    annotated["text_regions"] = json!([
        {"text":"SAMPLE","bbox_1000_xyxy":[30,700,300,950],"kind":"watermark","legibility":"readable"},
        {"text":"HELLO","bbox_1000_xyxy":[500,50,900,200],"kind":"overlay","legibility":"readable"}
    ]);
    annotated["visual_regions"] = json!([{"kind":"sticker","bbox_1000_xyxy":[800,800,950,950],"description":"A small heart-shaped decoration.","relation":"Overlapping the lower-right background."}]);
    let saved = ok(&s, "work_annotate", leased_input(&item, "boxed", annotated));
    assert_eq!(saved["next_stage"], "completed");
    assert_eq!(saved["annotation_details"]["text_region_count"], 2);
    let current = f.repo.media_by_id(m.id).unwrap().unwrap();
    assert_eq!(current.sha256, m.sha256);
    assert!(current.edit.is_none());
}

#[test]
fn final_overlay_inventory_and_an_agent_chosen_preset_cannot_be_skipped() {
    let (mut f, settings, m) = fixture();
    let (s, item, _) = start_work(&f, m.id, "curate");
    ok(&s, "work_view", leased_input(&item, "v", json!({})));
    let mut report = serde_json::to_value(assessment(&m, Decision::Keep)).unwrap();
    for key in ["media_id", "expected_sha256", "retained_elements"] {
        report.as_object_mut().unwrap().remove(key);
    }
    assert_eq!(
        s.execute("work_assess", leased_input(&item, "no-inventory", report))
            .metadata["code"],
        "retained_elements_required"
    );
    // Standalone crop proposals obey the same fixed ratio contract.
    ok(&s, "work_release", item);
    view(&mut f.repo, &settings, m.id);
    let mut args = crop_args(&m, false);
    args.ratio = None;
    assert_eq!(
        tools::error_result(tools::crop(&mut f.repo, &settings, args).unwrap_err()).metadata
            ["code"],
        "crop_ratio_required"
    );
}

#[test]
fn text_changes_after_planning_are_never_overwritten_even_with_authorized_overwrite() {
    let (f, _, m) = fixture();
    let s = service(&f, true);
    ok(
        &s,
        "job_submit",
        json!({"request_id":"protect","mode":"annotate","media_ids":[m.id],"outputs":["caption"],"overwrite":true}),
    );
    ok(&s, "jobs_tick", json!({}));
    let item = ok(&s, "work_claim", json!({"owner":"test"}))["items"][0].clone();
    ok(&s, "work_view", leased_input(&item, "v", json!({})));
    fs::write(m.path.with_extension("txt"), "New manual edit.").unwrap();
    let response = s.execute(
        "work_annotate",
        leased_input(
            &item,
            "a",
            json!({"caption":"Old model suggestion.","reason":"Test","model":"test"}),
        ),
    );
    assert_eq!(response.metadata["code"], "annotation_conflict");
    assert_eq!(
        fs::read_to_string(m.path.with_extension("txt")).unwrap(),
        "New manual edit."
    );
}

#[test]
fn submission_replay_keeps_original_scope_after_job_changes_and_validation_receipt_stays_error() {
    let (f, _, m) = fixture();
    let (s, item, _) = start_work(&f, m.id, "annotate");
    ok(&s, "work_view", leased_input(&item, "v", json!({})));
    let invalid = leased_input(
        &item,
        "bad",
        json!({"caption":"","reason":"Test","model":"test"}),
    );
    assert!(s.execute("work_annotate", invalid.clone()).is_error);
    assert!(s.execute("work_annotate", invalid).is_error);
    ok(
        &s,
        "work_annotate",
        leased_input(
            &item,
            "save",
            json!({"caption":"A blue square.","reason":"Test","model":"test"}),
        ),
    );
    assert_eq!(
        ok(
            &s,
            "job_submit",
            json!({"request_id":"job-1","media_ids":[m.id],"mode":"annotate","outputs":["caption"]})
        )["replayed"],
        true
    );
}

#[test]
fn queued_download_reuses_existing_collection_and_is_idempotent_without_network() {
    let (f, _, _) = fixture();
    let s = service(&f, true);
    let input = json!({"request_id":"download-1","collection_id":f.collection,"source":"gelbooru","minimum_short_side":800});
    let submitted = ok(&s, "download_submit", input.clone());
    assert_eq!(
        ok(&s, "download_submit", input)["download_id"],
        submitted["download_id"]
    );
    let id = submitted["download_id"].as_i64().unwrap();
    ok(
        &s,
        "download_control",
        json!({"download_id":id,"action":"pause"}),
    );
    assert_eq!(
        ok(&s, "download_status", json!({"download_id":id}))["state"],
        "paused"
    );
    ok(
        &s,
        "download_control",
        json!({"download_id":id,"action":"resume"}),
    );
    assert_eq!(
        ok(&s, "download_status", json!({"download_id":id}))["state"],
        "queued"
    );
    assert_eq!(f.repo.list_jobs().unwrap().len(), 1);
}

#[test]
fn complete_annotation_is_skipped_and_job_status_pages_keep_full_counts() {
    let (f, _, m) = fixture();
    let (s, item, _) = start_work(&f, m.id, "annotate");
    ok(&s, "work_view", leased_input(&item, "v", json!({})));
    ok(
        &s,
        "work_annotate",
        leased_input(
            &item,
            "save",
            json!({"caption":"A flat blue square.","reason":"Test","model":"test"}),
        ),
    );
    let submitted = ok(
        &s,
        "job_submit",
        json!({"request_id":"skip","media_ids":[m.id],"mode":"annotate","outputs":["caption"]}),
    );
    ok(&s, "jobs_tick", json!({}));
    let state = ok(&s, "job_status", json!({"job_id":submitted["job_id"]}));
    assert_eq!(state["counts"]["completed"], 1);
    assert!(ok(
        &s,
        "work_claim",
        json!({"owner":"test","job_id":submitted["job_id"]})
    )["items"]
        .as_array()
        .unwrap()
        .is_empty());
    // Synthetic catalog rows exercise pagination without downloading or writing
    // hundreds of image copies. Only list/status operations use these rows.
    let mut ids = vec![m.id];
    for index in 0..129 {
        f.repo
            .connection
            .execute(
                "INSERT INTO media(md5,metadata,keep) SELECT ?2,metadata,0 FROM media WHERE id=?1",
                rusqlite::params![m.id, format!("synthetic-page-{index}")],
            )
            .unwrap();
        let id = f.repo.connection.last_insert_rowid();
        f.repo
            .connection
            .execute(
                "UPDATE media SET metadata=json_set(metadata,'$.id',?1) WHERE id=?1",
                [id],
            )
            .unwrap();
        ids.push(id);
    }
    let submitted = ok(
        &s,
        "job_submit",
        json!({"request_id":"pages","media_ids":ids,"mode":"bucket"}),
    );
    let mut after = 0;
    let mut seen = BTreeSet::new();
    loop {
        let page = ok(
            &s,
            "job_status",
            json!({"job_id":submitted["job_id"],"after_id":after,"limit":17}),
        );
        assert_eq!(page["counts"]["local"], 130);
        assert!(page["plan"]["items"].is_null());
        let rows = page["items"].as_array().unwrap();
        assert!(rows.len() <= 17);
        for row in rows {
            assert!(seen.insert(row["work_id"].as_i64().unwrap()));
        }
        let Some(next) = page["next_after_id"].as_i64() else {
            break;
        };
        after = next;
    }
    assert_eq!(seen.len(), 130);
}

#[test]
fn export_job_runs_one_batch_and_reports_ineligible_images_as_deferred() {
    let (f, _, m) = fixture();
    let s = service(&f, true);
    let destination = f.directory.path().join("flat-export");
    let submitted = ok(
        &s,
        "job_submit",
        json!({"request_id":"export","mode":"export","media_ids":[m.id],"destination":destination,"text_kind":"caption"}),
    );
    ok(&s, "jobs_tick", json!({}));
    let state = ok(&s, "job_status", json!({"job_id":submitted["job_id"]}));
    assert_eq!(state["counts"]["deferred"], 1);
    assert_eq!(state["items"][0]["result"]["exported"], false);
    assert_eq!(
        f.repo
            .list_workflows()
            .unwrap()
            .iter()
            .filter(|j| j.kind == "export")
            .count(),
        1
    );
    assert!(!destination.join(format!("media_{}.png", m.id)).exists());
}
fn view(repo: &mut SqliteCatalog, settings: &Settings, id: i64) {
    tools::view(
        repo,
        settings,
        ViewArgs {
            media_id: id,
            original: false,
            bbox_1000_xyxy: None,
        },
    )
    .unwrap();
}
fn crop_args(m: &bdm_domain::ingest::StoredMedia, apply: bool) -> CropArgs {
    CropArgs {
        media_id: m.id,
        expected_sha256: m.sha256.clone(),
        bbox_1000_xyxy: [20, 20, 980, 980],
        ratio: Some("1:1".into()),
        preview_sha256: None,
        apply,
        reason: "Retain the complete subject with balanced margins".into(),
        model: "unit-test".into(),
    }
}
fn assessment(m: &bdm_domain::ingest::StoredMedia, decision: Decision) -> AssessArgs {
    AssessArgs {
        media_id: m.id,
        expected_sha256: m.sha256.clone(),
        aesthetic: Some(8.0),
        training_suitability: Some(9.0),
        composition: Some(8.0),
        detail: Some(9.0),
        watermark_present: None,
        retained_elements: Some(Vec::new()),
        style: "textured illustration".into(),
        decision,
        stage: AssessmentStage::Final,
        crop_candidate: false,
        crop_outcome: None,
        reason: "Clear retained detail and composition".into(),
        model: "unit-test".into(),
    }
}

#[test]
fn crop_requires_actual_view_and_preview_keeps_native_pixels_and_original_and_rejects_stale_hash() {
    let (mut f, settings, m) = fixture();
    let bytes = fs::read(&m.path).unwrap();
    assert!(tools::crop(&mut f.repo, &settings, crop_args(&m, true)).is_err());
    view(&mut f.repo, &settings, m.id);
    assert!(tools::crop(&mut f.repo, &settings, crop_args(&m, true)).is_err());
    tools::crop(&mut f.repo, &settings, crop_args(&m, false)).unwrap();
    assert_eq!(fs::read(&m.path).unwrap(), bytes);
    tools::crop(&mut f.repo, &settings, crop_args(&m, true)).unwrap();
    let saved = f.repo.media_by_id(m.id).unwrap().unwrap();
    assert_eq!(saved.dimensions(), (768, 768));
    assert_eq!(saved.current_extension(), "png");
    let original = image::load_from_memory(&bytes).unwrap();
    assert_eq!(
        image::open(&saved.path).unwrap().to_rgba8(),
        original.crop_imm(16, 16, 768, 768).to_rgba8()
    );
    assert_eq!(
        fs::read(
            &saved
                .edit
                .as_ref()
                .unwrap()
                .source_archive
                .as_ref()
                .unwrap()
                .path
        )
        .unwrap(),
        bytes
    );
    assert!(!saved.path.parent().unwrap().join(".edit-source").exists());
    assert!(tools::crop(&mut f.repo, &settings, crop_args(&m, true)).is_err());
    assert!(tools::assess(&mut f.repo, &settings, assessment(&saved, Decision::Keep)).is_err());
    view(&mut f.repo, &settings, m.id);
    tools::assess(&mut f.repo, &settings, assessment(&saved, Decision::Keep)).unwrap();
}

#[test]
fn annotations_use_local_identity_format_preserve_source_manual_bytes_and_quality() {
    let (mut f, settings, m) = fixture();
    view(&mut f.repo, &settings, m.id);
    let raw = fs::read(m.path.with_extension("raw_tags.txt")).unwrap();
    tools::assess(&mut f.repo, &settings, assessment(&m, Decision::Keep)).unwrap();
    let mut args = AnnotationArgs {
        media_id: m.id,
        expected_sha256: m.sha256.clone(),
        caption: Some("A colorful landscape with textured strokes.".into()),
        tags: Some(vec![
            "scenery".into(),
            "masterpiece".into(),
            "unknown tag".into(),
        ]),
        text_regions: Some(vec![]),
        special_elements: Some(vec![]),
        visual_regions: None,
        review_reasons: None,
        require_final_assessment: false,
        overwrite: false,
        reason: "Caption grounded in the actual image".into(),
        model: "unit-test".into(),
    };
    tools::annotate(&mut f.repo, &settings, args.clone()).unwrap();
    let caption = fs::read_to_string(m.path.with_extension("txt")).unwrap();
    assert_eq!(
        caption,
        "@artist, character, A colorful landscape with textured strokes."
    );
    let tags = fs::read_to_string(m.path.with_extension("tags.txt")).unwrap();
    assert_eq!(tags, "@artist, character, scenery");
    let score: f64 = f
        .repo
        .connection
        .query_row(
            "SELECT aesthetic FROM media_quality WHERE media_id=?1",
            [m.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(score, 8.0);
    let manual = b"\xef\xbb\xbfManual TXT\r\n\xff";
    fs::write(m.path.with_extension("txt"), manual).unwrap();
    args.caption = Some("A different factual description.".into());
    tools::annotate(&mut f.repo, &settings, args.clone()).unwrap();
    assert_eq!(fs::read(m.path.with_extension("txt")).unwrap(), manual);
    args.overwrite = true;
    tools::annotate(&mut f.repo, &settings, args).unwrap();
    assert!(fs::read_to_string(m.path.with_extension("txt"))
        .unwrap()
        .ends_with("A different factual description."));
    assert_eq!(
        fs::read(m.path.with_extension("raw_tags.txt")).unwrap(),
        raw
    );
    assert_eq!(f.repo.media_by_id(m.id).unwrap().unwrap().post, m.post);
}

#[test]
fn rejection_never_deletes_and_human_cancel_or_override_persists() {
    let (mut f, settings, m) = fixture();
    view(&mut f.repo, &settings, m.id);
    tools::assess(&mut f.repo, &settings, assessment(&m, Decision::Reject)).unwrap();
    let db = f.directory.path().join("catalog.sqlite3");
    assert!(snapshot(&db).unwrap().rejected.contains(&(m.id as usize)));
    assert!(m.path.exists());
    assert_eq!(crate::editing::hash(&m.path).unwrap(), m.sha256);
    f.repo
        .clear_agent_rejections(&BTreeSet::from([m.id]))
        .unwrap();
    assert!(snapshot(&db).unwrap().rejected.is_empty());
    tools::assess(&mut f.repo, &settings, assessment(&m, Decision::Reject)).unwrap();
    f.repo.set_media_keep(m.id, true).unwrap();
    assert!(snapshot(&db).unwrap().rejected.is_empty());
    let mut invalid = assessment(&m, Decision::Keep);
    invalid.aesthetic = Some(99.0);
    assert!(tools::assess(&mut f.repo, &settings, invalid).is_err());
}

#[test]
fn duplicate_scan_cannot_auto_delete_and_final_review_preserves_associations() {
    use image::ImageEncoder;
    let (mut f, settings, first) = fixture();
    let decoded = image::load_from_memory(&f.provider.bytes)
        .unwrap()
        .to_rgb8();
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut bytes,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::NoFilter,
    )
    .write_image(decoded.as_raw(), 800, 800, image::ExtendedColorType::Rgb8)
    .unwrap();
    let collection = f
        .repo
        .add(
            bdm_domain::catalog::CollectionKind::Character,
            &bdm_domain::catalog::SourceTag {
                provider: bdm_domain::sources::SourceProvider::Danbooru,
                id: 2,
                name: "other_character".into(),
                category: 4,
                post_count: 1,
            },
        )
        .unwrap()
        .record
        .id;
    f.provider.bytes = bytes;
    f.post.id += 1;
    f.post.file_size = f.provider.bytes.len() as u64;
    f.post.md5 = format!("{:x}", md5::Md5::digest(&f.provider.bytes));
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            collection,
            800,
            &AtomicBool::new(false),
            &f.provider,
        )
        .unwrap();
    let second = f
        .repo
        .list_media()
        .unwrap()
        .into_iter()
        .find(|m| m.id != first.id)
        .unwrap();
    let mut vector = vec![0.0f32; 384];
    vector[0] = 1.0;
    for m in [&first, &second] {
        let hash = crate::workflows::phash(&m.path).unwrap();
        f.repo
            .connection
            .execute(
                "INSERT OR REPLACE INTO visual_features VALUES(?1,?2,?3,?4)",
                rusqlite::params![
                    m.id,
                    crate::visual::VERSION,
                    format!("{hash:016x}"),
                    serde_json::to_string(&vector).unwrap()
                ],
            )
            .unwrap();
    }
    tools::scan(
        &mut f.repo,
        &settings,
        ScanArgs {
            media_ids: vec![first.id, second.id],
            similarity: Some(0.95),
        },
    )
    .unwrap();
    assert_eq!(f.repo.list_media().unwrap().len(), 2);
    assert!(first.path.exists() && second.path.exists());
    assert!(f
        .repo
        .list_candidates()
        .unwrap()
        .iter()
        .any(|p| p.dual_confirmed() && p.state == "pending"));
    view(&mut f.repo, &settings, first.id);
    view(&mut f.repo, &settings, second.id);
    tools::pair(
        &mut f.repo,
        &settings,
        PairArgs {
            first_id: first.id,
            second_id: second.id,
            expected_sha256: [first.sha256.clone(), second.sha256.clone()],
            survivor_id: Some(first.id),
            reason: "Same pixels in different source encodings".into(),
            model: "unit-test".into(),
        },
    )
    .unwrap();
    assert_eq!(f.repo.list_media().unwrap().len(), 2);
    assert!(f
        .repo
        .candidate_rejected(first.id.min(second.id), first.id.max(second.id))
        .unwrap());
    assert!(f
        .repo
        .delete_review_scope(&BTreeSet::from([first.id, second.id]))
        .is_err());
    f.repo
        .delete_review_scope(&BTreeSet::from([second.id]))
        .unwrap();
    let retained = f.repo.media_by_id(first.id).unwrap().unwrap();
    assert!(retained.collections.contains(&collection));
    assert!(retained.collections.contains(&f.collection));
}

#[test]
fn read_only_and_library_boundary_are_enforced_and_mcp_tools_are_discoverable() {
    let (mut f, mut settings, m) = fixture();
    view(&mut f.repo, &settings, m.id);
    settings.writes = false;
    assert!(tools::assess(&mut f.repo, &settings, assessment(&m, Decision::Keep)).is_err());
    settings.library_root = f.directory.path().join("unrelated");
    fs::create_dir(&settings.library_root).unwrap();
    assert!(tools::view(
        &mut f.repo,
        &settings,
        ViewArgs {
            media_id: m.id,
            original: false,
            bbox_1000_xyxy: None
        }
    )
    .is_err());
    let server = AgentServer::open(
        &f.directory.path().join("catalog.sqlite3"),
        f.directory.path(),
        &f.directory.path().join("library"),
        false,
    )
    .unwrap();
    assert!(server
        .tool_router
        .list_all()
        .iter()
        .any(|t| t.name == "bdm_agent"));
}

#[test]
fn media_mutation_lock_is_an_os_file_lock_not_only_a_process_mutex() {
    let f = crate::workflow_tests::fixture();
    let lock = f.repo.media_lock.clone();
    let guard = lock.lock().unwrap();
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.directory.path().join("catalog.media.lock"))
        .unwrap();
    assert!(fs2::FileExt::try_lock_exclusive(&file).is_err());
    drop(guard);
    fs2::FileExt::try_lock_exclusive(&file).unwrap();
}

#[test]
fn collection_and_image_pages_are_bounded_and_can_be_traversed_without_dropped_ids() {
    fn text(result: ServiceReply) -> serde_json::Value {
        result.metadata
    }
    let (mut f, settings, m) = fixture();
    let mut after = 0;
    let mut seen = BTreeSet::new();
    loop {
        let page = text(
            tools::collections(
                &mut f.repo,
                &settings,
                ListArgs {
                    limit: Some(1),
                    after_id: after,
                    collection: None,
                },
            )
            .unwrap(),
        );
        assert!(page.to_string().len() < 8192);
        let rows = page["collections"].as_array().unwrap();
        if rows.is_empty() {
            break;
        }
        assert_eq!(rows.len(), 1);
        assert!(seen.insert(rows[0]["id"].as_i64().unwrap()));
        after = page["next_after_id"].as_i64().unwrap();
    }
    assert_eq!(seen.len(), f.repo.list().unwrap().len());
    let page = text(tools::list(&mut f.repo, &settings, ListArgs::default()).unwrap());
    assert_eq!(page["images"][0]["id"], m.id);
    assert_eq!(page["next_after_id"], m.id);
}

fn payload(result: &ServiceReply) -> serde_json::Value {
    result.metadata.clone()
}

fn annotation_args(m: &bdm_domain::ingest::StoredMedia) -> AnnotationArgs {
    AnnotationArgs {
        media_id: m.id,
        expected_sha256: m.sha256.clone(),
        caption: Some("A landscape painted with textured strokes.".into()),
        tags: Some(vec!["scenery".into()]),
        text_regions: None,
        special_elements: None,
        visual_regions: None,
        review_reasons: None,
        require_final_assessment: false,
        overwrite: false,
        reason: "Inspect visible details".into(),
        model: "unit-test".into(),
    }
}

#[test]
fn annotation_rejections_explain_exact_terms_and_do_not_discard_caption_or_leak_source_metadata() {
    let (mut f, settings, mut m) = fixture();
    m.post.tag_string.push_str(" highres commentary_request");
    f.repo
        .connection
        .execute(
            "UPDATE media SET metadata=?1 WHERE id=?2",
            rusqlite::params![serde_json::to_string(&m).unwrap(), m.id],
        )
        .unwrap();
    view(&mut f.repo, &settings, m.id);
    let mut args = annotation_args(&m);
    args.tags = Some(vec![
        "invented visual term".into(),
        "masterpiece".into(),
        "artist".into(),
    ]);
    let response = payload(&tools::annotate(&mut f.repo, &settings, args.clone()).unwrap());
    assert_eq!(response["status"], "partial");
    assert_eq!(response["saved"], serde_json::json!(["caption"]));
    assert_eq!(response["tag_rejections"][0]["code"], "unknown_category");
    assert_eq!(response["tag_rejections"][1]["code"], "non_visual_metadata");
    assert_eq!(
        response["tag_rejections"][2]["code"],
        "identity_merged_locally"
    );
    assert!(response["tag_vocabulary"]["source_general"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("scenery")));
    assert_eq!(response["verification"]["sidecars_match"], true);
    assert_eq!(
        response["readback"][0]["content"],
        fs::read_to_string(m.path.with_extension("txt")).unwrap()
    );
    // A bad *unused* replacement caption must not prevent saving valid tags.
    args.caption = Some("@artist: this replacement must be preserved, not validated".into());
    args.tags = Some(vec!["scenery".into(), "unknown term".into()]);
    let response = payload(&tools::annotate(&mut f.repo, &settings, args).unwrap());
    assert_eq!(response["preserved"], serde_json::json!(["caption"]));
    assert_eq!(response["saved"], serde_json::json!(["tags"]));
    assert_eq!(
        fs::read_to_string(m.path.with_extension("tags.txt")).unwrap(),
        "@artist, character, scenery"
    );
}

#[test]
fn all_rejected_tags_leave_existing_files_untouched_and_omitted_details_are_preserved() {
    let (mut f, settings, m) = fixture();
    view(&mut f.repo, &settings, m.id);
    let regions = vec![crate::ai::TextRegion {
        text: "unreadable text".into(),
        bbox_1000_xyxy: [20, 20, 100, 50],
        kind: "watermark".into(),
        legibility: "unreadable".into(),
        ..Default::default()
    }];
    f.repo
        .save_annotation_details(
            m.id,
            &crate::ai::AnnotationDetails {
                schema_version: "test".into(),
                text_regions: regions.clone(),
                special_elements: vec!["cross_section".into()],
                user_hint: None,
                ..Default::default()
            },
        )
        .unwrap();
    let mut args = annotation_args(&m);
    args.caption = None;
    args.tags = Some(vec!["unknown term".into()]);
    let old = fs::read(m.path.with_extension("tags.txt")).unwrap();
    let response = payload(&tools::annotate(&mut f.repo, &settings, args.clone()).unwrap());
    assert_eq!(response["status"], "needs_correction");
    assert!(response["saved"].as_array().unwrap().is_empty());
    assert_eq!(fs::read(m.path.with_extension("tags.txt")).unwrap(), old);
    args.tags = Some(vec!["scenery".into()]);
    tools::annotate(&mut f.repo, &settings, args.clone()).unwrap();
    let details = f.repo.list_annotation_details().unwrap();
    assert_eq!(details[&m.id].text_regions, regions);
    assert_eq!(details[&m.id].special_elements, vec!["cross_section"]);
    args.overwrite = true;
    args.text_regions = Some(vec![]);
    tools::annotate(&mut f.repo, &settings, args).unwrap();
    let details = f.repo.list_annotation_details().unwrap();
    assert!(details[&m.id].text_regions.is_empty());
    assert_eq!(details[&m.id].special_elements, vec!["cross_section"]);
}

#[test]
fn view_supplies_usable_vocabulary_and_crop_errors_include_a_recovery_step() {
    let (mut f, settings, m) = fixture();
    let response = payload(
        &tools::view(
            &mut f.repo,
            &settings,
            ViewArgs {
                media_id: m.id,
                original: false,
                bbox_1000_xyxy: None,
            },
        )
        .unwrap(),
    );
    assert!(response["tag_vocabulary"]["source_general"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("scenery")));
    assert!(response["next"].is_object());
    let error = tools::crop(&mut f.repo, &settings, crop_args(&m, true)).unwrap_err();
    let error = tools::error_result(error);
    assert!(error.is_error);
    assert_eq!(payload(&error)["code"], "crop_preview_required");
    assert_eq!(payload(&error)["next"]["tool"], "bdm_crop");
    assert_eq!(payload(&error)["next"]["args"]["apply"], false);
}

#[test]
fn compound_regions_roundtrip_to_dataset_index_and_preserve_review_history() {
    let (mut f, settings, m) = fixture();
    view(&mut f.repo, &settings, m.id);
    let mut args = annotation_args(&m);
    args.text_regions = Some(vec![crate::ai::TextRegion {
        text: "HELLO SPRING".into(),
        bbox_1000_xyxy: [30, 40, 390, 150],
        kind: "overlay".into(),
        legibility: "readable".into(),
        language: Some("en".into()),
        appearance: Some("Pale blue uppercase lettering.".into()),
        confidence: Some(0.8),
    }]);
    args.visual_regions = Some(vec![crate::ai::VisualRegion {
        kind: "detail_inset".into(),
        bbox_1000_xyxy: [700, 700, 980, 980],
        description: "An enlarged view of the flower.".into(),
        relation: "Separate framed inset at the lower right.".into(),
        confidence: None,
    }]);
    let response = payload(&tools::annotate(&mut f.repo, &settings, args).unwrap());
    assert_eq!(response["annotation_details"]["visual_region_count"], 1);
    f.repo.set_media_keep(m.id, true).unwrap();
    f.repo.review_artifact(m.id, "caption").unwrap();
    let raw = fs::read(m.path.with_extension("raw_tags.txt")).unwrap();
    let mut job = f.repo.new_workflow("export", vec![m.id]).unwrap();
    job.mode = "caption".into();
    job.export_layout = "flat".into();
    let root = f.directory.path().join("export");
    crate::export::run(
        &mut f.repo,
        &mut job,
        &root,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let index_path = root.join("annotations.jsonl");
    let original = fs::read(&index_path).unwrap();
    let record: serde_json::Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(
        record["annotation_details"]["text_regions"][0]["confidence"],
        0.8
    );
    assert_eq!(
        record["annotation_details"]["visual_regions"][0]["kind"],
        "detail_inset"
    );
    assert_eq!(record["annotation_matches_source_current"], true);
    assert!(record["history"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["actor"] == "human"));
    assert!(record["history"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["data"]["annotation_details"]["visual_regions"][0]["kind"] == "detail_inset"));
    // Resume and repeated export update one row, not one duplicate per run.
    crate::export::run(
        &mut f.repo,
        &mut job,
        &root,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&index_path).unwrap().lines().count(), 1);
    assert_eq!(
        fs::read(m.path.with_extension("raw_tags.txt")).unwrap(),
        raw
    );
    let receipt = root.join(".bdm-manifests/annotations-index.json");
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
    fs::write(&receipt,serde_json::to_vec(&serde_json::json!({"schema":"bdm-annotation-index-receipt-v1","sha256":"old-sha","pending_sha256":saved["sha256"]})).unwrap()).unwrap();
    crate::export::run(
        &mut f.repo,
        &mut job,
        &root,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    // A user-modified index cannot be silently replaced.
    fs::write(&index_path, b"manual user index\n").unwrap();
    assert!(crate::export::run(
        &mut f.repo,
        &mut job,
        &root,
        &AtomicBool::new(false),
        |_| {}
    )
    .is_err());
    assert_eq!(fs::read(&index_path).unwrap(), b"manual user index\n");
}

#[test]
fn partial_decorative_text_confidence_and_stale_region_hash_are_checked() {
    let (mut f, settings, m) = fixture();
    view(&mut f.repo, &settings, m.id);
    let mut args = annotation_args(&m);
    let mut text = crate::ai::TextRegion {
        text: "SPR".into(),
        bbox_1000_xyxy: [30, 40, 390, 150],
        kind: "sign".into(),
        legibility: "partial".into(),
        ..Default::default()
    };
    assert!(text.is_valid());
    text.confidence = Some(1.1);
    assert!(!text.is_valid());
    text.confidence = None;
    let mut decorative = text.clone();
    decorative.legibility = "decorative".into();
    assert!(!decorative.is_valid());
    decorative.text = "unreadable text".into();
    assert!(decorative.is_valid());
    args.text_regions = Some(vec![text, decorative]);
    tools::annotate(&mut f.repo, &settings, args.clone()).unwrap();
    let old_details = f.repo.list_annotation_details().unwrap()[&m.id].clone();
    tools::crop(&mut f.repo, &settings, crop_args(&m, false)).unwrap();
    tools::crop(&mut f.repo, &settings, crop_args(&m, true)).unwrap();
    let cropped = f.repo.media_by_id(m.id).unwrap().unwrap();
    // Cropping normally invalidates quality details. Simulate stale imported
    // region metadata to check that it cannot be silently relabelled as fresh.
    f.repo.save_annotation_details(m.id, &old_details).unwrap();
    view(&mut f.repo, &settings, m.id);
    args.expected_sha256 = cropped.sha256.clone();
    args.overwrite = true;
    args.text_regions = None;
    let error =
        tools::error_result(tools::annotate(&mut f.repo, &settings, args.clone()).unwrap_err());
    assert_eq!(payload(&error)["code"], "stale_annotation_details");
    args.text_regions = Some(vec![]);
    args.visual_regions = Some(vec![]);
    args.special_elements = Some(vec![]);
    tools::annotate(&mut f.repo, &settings, args).unwrap();
    assert_eq!(
        f.repo.list_annotation_details().unwrap()[&m.id].image_sha256,
        Some(cropped.sha256)
    );
}

#[test]
fn uncertain_content_defers_training_without_deleting_originals() {
    let (mut f, settings, m) = fixture();
    f.repo.set_media_keep(m.id, true).unwrap();
    view(&mut f.repo, &settings, m.id);
    let mut args = annotation_args(&m);
    args.caption = None;
    args.tags = None;
    args.review_reasons = Some(vec![
        "Age cannot be determined reliably; manual review required.".into(),
    ]);
    let response = payload(&tools::annotate(&mut f.repo, &settings, args).unwrap());
    assert_eq!(response["status"], "review_saved");
    assert_eq!(response["next"]["action"], "human_review_queued");
    assert!(!f.repo.media_by_id(m.id).unwrap().unwrap().keep);
    assert_eq!(
        f.repo.agent_review_value(m.id).unwrap().unwrap()["decision"],
        "defer"
    );
    assert!(m.path.is_file());
    let mut preliminary = assessment(&m, Decision::Defer);
    preliminary.stage = AssessmentStage::Preliminary;
    preliminary.crop_candidate = true;
    assert!(tools::assess(&mut f.repo, &settings, preliminary).is_err());
    assert!(
        !f.repo.agent_review_value(m.id).unwrap().unwrap()["report"]["review_reasons"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let error = tools::assess(&mut f.repo, &settings, assessment(&m, Decision::Keep)).unwrap_err();
    assert_eq!(
        payload(&tools::error_result(error))["code"],
        "human_review_required"
    );
}

#[test]
fn crop_first_candidate_cannot_reject_or_annotate_until_final_crop_assessment() {
    let (mut f, settings, m) = fixture();
    f.repo.set_media_keep(m.id, true).unwrap();
    view(&mut f.repo, &settings, m.id);
    let mut initial = assessment(&m, Decision::Reject);
    initial.stage = AssessmentStage::Preliminary;
    initial.crop_candidate = true;
    initial.aesthetic = Some(3.0);
    let response = payload(&tools::assess(&mut f.repo, &settings, initial.clone()).unwrap());
    assert_eq!(response["decision"], "defer");
    assert_eq!(response["queued_for_deletion"], false);
    assert_eq!(response["next"]["tool"], "bdm_crop");
    assert!(!f.repo.media_by_id(m.id).unwrap().unwrap().keep);
    assert!(snapshot(&f.directory.path().join("catalog.sqlite3"))
        .unwrap()
        .rejected
        .is_empty());
    let error =
        tools::assess(&mut f.repo, &settings, assessment(&m, Decision::Reject)).unwrap_err();
    assert_eq!(
        payload(&tools::error_result(error))["code"],
        "crop_first_required"
    );
    initial.crop_candidate = false;
    assert!(tools::assess(&mut f.repo, &settings, initial).is_err());
    let mut annotation = annotation_args(&m);
    annotation.require_final_assessment = true;
    assert!(tools::annotate(&mut f.repo, &settings, annotation.clone()).is_err());
    tools::crop(&mut f.repo, &settings, crop_args(&m, false)).unwrap();
    tools::crop(&mut f.repo, &settings, crop_args(&m, true)).unwrap();
    let cropped = f.repo.media_by_id(m.id).unwrap().unwrap();
    annotation.expected_sha256 = cropped.sha256.clone();
    view(&mut f.repo, &settings, m.id);
    assert!(tools::annotate(&mut f.repo, &settings, annotation.clone()).is_err());
    let mut final_result = assessment(&cropped, Decision::Keep);
    final_result.crop_outcome = Some(CropOutcome::Applied);
    tools::assess(&mut f.repo, &settings, final_result).unwrap();
    let response = payload(&tools::annotate(&mut f.repo, &settings, annotation).unwrap());
    assert_eq!(response["status"], "saved");
    assert!(f.repo.media_by_id(m.id).unwrap().unwrap().keep);
    let mut export = f.repo.new_workflow("export", vec![m.id]).unwrap();
    export.mode = "caption".into();
    export.export_layout = "flat".into();
    let destination = f.directory.path().join("curated-export");
    crate::export::run(
        &mut f.repo,
        &mut export,
        &destination,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let row: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(destination.join("annotations.jsonl")).unwrap())
            .unwrap();
    assert!(row["history"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["action"] == "agent_assessment" && e["data"]["stage"] == "preliminary"));
    assert!(row["history"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["action"] == "agent_assessment" && e["data"]["stage"] == "final"));
}

#[test]
fn unhelpful_crop_requires_preview_then_final_rejection_removes_training_approval() {
    let (mut f, settings, m) = fixture();
    view(&mut f.repo, &settings, m.id);
    let original = fs::read(&m.path).unwrap();
    let mut initial = assessment(&m, Decision::Defer);
    initial.stage = AssessmentStage::Preliminary;
    initial.crop_candidate = true;
    tools::assess(&mut f.repo, &settings, initial).unwrap();
    let mut final_result = assessment(&m, Decision::Reject);
    final_result.crop_outcome = Some(CropOutcome::NotImproved);
    assert!(tools::assess(&mut f.repo, &settings, final_result.clone()).is_err());
    tools::crop(&mut f.repo, &settings, crop_args(&m, false)).unwrap();
    let response = payload(&tools::assess(&mut f.repo, &settings, final_result).unwrap());
    assert_eq!(response["queued_for_deletion"], true);
    assert!(!f.repo.media_by_id(m.id).unwrap().unwrap().keep);
    assert_eq!(fs::read(&m.path).unwrap(), original);
    f.repo.set_media_keep(m.id, true).unwrap();
    tools::assess(&mut f.repo, &settings, assessment(&m, Decision::Reject)).unwrap();
    assert!(!f.repo.media_by_id(m.id).unwrap().unwrap().keep);
}

#[test]
fn preliminary_score_and_state_roll_back_together_if_review_commit_fails() {
    let (mut f, settings, m) = fixture();
    view(&mut f.repo, &settings, m.id);
    f.repo.connection.execute_batch("CREATE TRIGGER fail_assessment_commit BEFORE UPDATE OF keep ON media BEGIN SELECT RAISE(ABORT,'test rollback'); END;").unwrap();
    let mut initial = assessment(&m, Decision::Keep);
    initial.stage = AssessmentStage::Preliminary;
    initial.crop_candidate = true;
    assert!(tools::assess(&mut f.repo, &settings, initial).is_err());
    assert!(f.repo.agent_review_value(m.id).unwrap().is_none());
}
