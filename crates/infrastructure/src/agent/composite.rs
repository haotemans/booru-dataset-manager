//! Fewer client round trips, without skipping existing visual or write gates.
//! Compound writes are ordered/recoverable, not an all-or-nothing transaction.
use super::{
    control_params::*,
    jobs,
    params::Decision,
    service::{AgentService, ServiceReply},
};
use crate::{catalog::SqliteCatalog, ingest::storage};
use bdm_ports::IngestError;
use rusqlite::{params, OptionalExtension};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn error(code: &str, message: &str) -> IngestError {
    IngestError::StorageDetails(json!({"code":code,"message":message}).to_string())
}
fn parse<T: DeserializeOwned>(input: Value) -> Result<T, IngestError> {
    serde_json::from_value(input).map_err(|e| error("invalid_arguments", &e.to_string()))
}
fn digest(value: &Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}
fn lease(input: &Value) -> Value {
    json!({"work_id":input["work_id"],"lease_token":input["lease_token"],"generation":input["generation"]})
}
fn child(input: &Value, phase: &str, payload: Value) -> Value {
    let mut call = lease(input);
    call["expected_sha256"] = input["expected_sha256"].clone();
    call["request_id"] = json!(format!(
        "finish:{}",
        digest(&json!([input["work_id"], input["request_id"], phase]))
    ));
    call["payload"] = payload;
    call
}

pub(super) fn dispatch(
    service: &AgentService,
    repo: &mut SqliteCatalog,
    operation: &str,
    input: Value,
) -> Result<ServiceReply, IngestError> {
    if !service.settings.writes {
        return Err(error("read_only", "Restart with --allow-writes"));
    }
    match operation {
        "work_next" => next(service, parse(input)?),
        "work_apply_view" => apply_view(service, parse(input)?),
        "work_finish" => finish(service, repo, parse(input)?),
        _ => Err(error("unknown_operation", operation)),
    }
}

fn view_current(
    service: &AgentService,
    item: &Value,
    mut preview: super::preview::PreviewOptions,
) -> ServiceReply {
    // A composite response always contains real pixels, even on a replay.
    preview.known_preview_token = None;
    let mut input = lease(item);
    input["payload"] = json!({"original":false,"preview":preview});
    let mut viewed = service.execute("work_view", input);
    let mut work = lease(item);
    work["expected_sha256"] = item["expected_sha256"].clone();
    if !viewed.is_error {
        work["expected_sha256"] = viewed.metadata["sha256"].clone();
    }
    viewed.metadata["work"] = work;
    viewed
}

fn next(service: &AgentService, args: WorkNextArgs) -> Result<ServiceReply, IngestError> {
    args.preview.validate()?;
    if args.job_id.is_some_and(|id| id <= 0) {
        return Err(error(
            "invalid_scope",
            "Use a positive job_id or omit it for the authorized queue",
        ));
    }
    let claimed=service.execute("work_claim",json!({"owner":args.owner,"job_id":args.job_id,"ttl_seconds":args.ttl_seconds,"limit":1,"request_id":args.request_id}));
    if claimed.is_error
        || claimed.metadata["items"]
            .as_array()
            .is_none_or(|a| a.is_empty())
    {
        return Ok(claimed);
    }
    let item = &claimed.metadata["items"][0];
    let mut viewed = view_current(service, item, args.preview);
    viewed.metadata["claim"] = claimed.metadata;
    Ok(viewed)
}

fn apply_view(
    service: &AgentService,
    args: WorkApplyViewArgs,
) -> Result<ServiceReply, IngestError> {
    args.payload.preview.validate()?;
    let input = json!(args);
    let mut apply = input.clone();
    apply["payload"] = input["payload"]["crop"].clone();
    let applied = service.execute("work_apply", apply);
    if applied.is_error {
        return Ok(applied);
    }
    let mut current = lease(&input);
    current["expected_sha256"] = applied.metadata["media"]["sha256"].clone();
    let mut viewed = view_current(service, &current, args.payload.preview);
    viewed.metadata["applied"] = json!(true);
    viewed.metadata["apply_result"] = applied.metadata;
    Ok(viewed)
}

fn begin_finish(
    repo: &mut SqliteCatalog,
    input: &Value,
) -> Result<Option<ServiceReply>, IngestError> {
    let id = input["work_id"]
        .as_i64()
        .filter(|i| *i > 0)
        .ok_or_else(|| error("invalid_arguments", "work_id"))?;
    let request = input["request_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
        .ok_or_else(|| error("invalid_arguments", "request_id"))?;
    let fingerprint = digest(&json!([
        "work_finish",
        input["expected_sha256"],
        input["payload"]
    ]));
    let proof = digest(&json!([input["lease_token"], input["generation"]]));
    let tx = storage(
        repo.connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
    )?;
    let previous:Option<(String,String,Option<String>,String)>=storage(tx.query_row("SELECT fingerprint,state,response,lease_proof FROM agent_receipts WHERE work_id=?1 AND request_id=?2",params![id,request],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional())?;
    if let Some((fp, state, response, previous_proof)) = previous {
        if proof != previous_proof {
            return Err(error(
                "lease_lost",
                "Finish receipt belongs to a different lease",
            ));
        }
        if fingerprint != fp {
            return Err(error("idempotency_conflict","Finish request_id reused with changed inputs; correct missing outputs through work_annotate with a new request ID"));
        }
        if state == "complete" {
            let metadata: Value = serde_json::from_str(&response.ok_or(IngestError::Storage)?)
                .map_err(|_| IngestError::Storage)?;
            let is_error = metadata["status"] == "error";
            return Ok(Some(ServiceReply {
                metadata,
                image: None,
                is_error,
            }));
        }
        if state != "bundle" {
            return Err(error(
                "recovery_required",
                "Inspect an interrupted write before retrying",
            ));
        }
    } else {
        let valid:bool=storage(tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_work w JOIN agent_jobs j ON w.job_id=j.id WHERE w.id=?1 AND w.lease=?2 AND w.generation=?3 AND w.expires>?4 AND w.busy=0 AND j.state='running' AND w.stage IN ('triage','crop','final'))",params![id,input["lease_token"].as_str(),input["generation"].as_i64(),jobs::now()],|r|r.get(0)))?;
        if !valid {
            return Err(error("wrong_stage_or_lease","Finish needs a live curation judgement lease. Annotation-stage corrections use work_annotate; inspect job status after a lost lease."));
        }
        storage(tx.execute(
            "INSERT INTO agent_receipts VALUES(?1,?2,?3,NULL,'bundle',?4)",
            params![id, request, fingerprint, proof],
        ))?;
    }
    storage(tx.commit())?;
    Ok(None)
}

fn child_matches_receipt(
    repo: &SqliteCatalog,
    call: &Value,
    reply: &ServiceReply,
) -> Result<bool, IngestError> {
    let raw:Option<String>=storage(repo.connection.query_row("SELECT response FROM agent_receipts WHERE work_id=?1 AND request_id=?2 AND state='complete'",params![call["work_id"].as_i64(),call["request_id"].as_str()],|r|r.get(0)).optional())?;
    let Some(raw) = raw else {
        return Ok(false);
    };
    let mut stored: Value = serde_json::from_str(&raw).map_err(|_| IngestError::Storage)?;
    let mut actual = reply.metadata.clone();
    // The service adds routing to errors after persisting their receipt. Compare
    // the actual outcome, not routing or a receipt completed by a racing caller.
    if let Some(object) = stored.as_object_mut() {
        object.remove("next");
    }
    if let Some(object) = actual.as_object_mut() {
        object.remove("next");
    }
    Ok(stored == actual)
}

fn finish(
    service: &AgentService,
    repo: &mut SqliteCatalog,
    args: WorkFinishArgs,
) -> Result<ServiceReply, IngestError> {
    if args.payload.assessment.crop_candidate {
        return Err(error("invalid_finish","Finish is for final judgement; propose/inspect/apply any needed crop first using work_assess/work_preview/work_apply_view"));
    }
    let keep = matches!(args.payload.assessment.decision, Decision::Keep);
    if keep != args.payload.annotation.is_some() {
        return Err(error(
            "invalid_finish",
            "KEEP requires annotation; REJECT/DEFER must not contain annotation",
        ));
    }
    let input = json!(args);
    if let Some(reply) = begin_finish(repo, &input)? {
        return Ok(reply);
    }
    let assessment_call = child(&input, "assessment", input["payload"]["assessment"].clone());
    let assessment = service.execute("work_assess", assessment_call.clone());
    let assessment_ok = !assessment.is_error;
    let annotation_call = child(&input, "annotation", input["payload"]["annotation"].clone());
    let annotation = if assessment_ok && keep && assessment.metadata["next_stage"] == "annotation" {
        Some(service.execute("work_annotate", annotation_call.clone()))
    } else {
        None
    };
    let last = annotation.as_ref().unwrap_or(&assessment);
    let is_error = last.is_error;
    let (stage, job_id): (String, i64) = storage(repo.connection.query_row(
        "SELECT stage,job_id FROM agent_work WHERE id=?1",
        [input["work_id"].as_i64()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ))?;
    let mut metadata = json!({"status":if is_error{"error"}else if stage=="annotation"{"partial"}else{"ok"},"work_id":input["work_id"],"assessment_saved":assessment_ok,"annotation_attempted":annotation.is_some(),"next_stage":stage,"next":last.metadata["next"],"code":last.metadata["code"],"assessment":assessment.metadata,"annotation":annotation.as_ref().map(|r|&r.metadata)});
    if !is_error && matches!(stage.as_str(), "completed" | "review" | "deferred") {
        metadata["next"] = json!({"operation":"work_next","tool":"bdm_work_next","job_id":job_id,"required":["owner","request_id"],"hint":"Use a new request_id to advance within this authorized job. If no item is available, inspect job status instead of busy-polling."});
    }
    if is_error {
        metadata["hint"]=json!("Assessment and annotation are ordered separate writes. Keep any saved assessment/caption; follow next for the failed/missing step. Never claim both phases rolled back.");
    }
    // Only cache a completed child outcome. Busy/ambiguous/in-flight work stays
    // resumable through its existing receipts; never infer that it did not save.
    let can_cache = if annotation.is_some() {
        child_matches_receipt(repo, &annotation_call, last)?
    } else {
        child_matches_receipt(repo, &assessment_call, last)?
    };
    if can_cache {
        storage(repo.connection.execute("UPDATE agent_receipts SET state='complete',response=?3 WHERE work_id=?1 AND request_id=?2 AND state='bundle'",params![input["work_id"].as_i64(),input["request_id"].as_str(),metadata.to_string()]))?;
    }
    Ok(ServiceReply {
        metadata,
        image: None,
        is_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tests::{fixture, leased_input, ok, service};
    fn ready(
        f: &crate::workflow_tests::Fixture,
        ids: Vec<i64>,
        mode: &str,
        outputs: Value,
    ) -> (AgentService, i64) {
        let s = service(f, true);
        let job = ok(
            &s,
            "job_submit",
            json!({"request_id":"composite-job","media_ids":ids,"mode":mode,"outputs":outputs}),
        )["job_id"]
            .as_i64()
            .unwrap();
        for _ in 0..3 {
            ok(&s, "jobs_tick", json!({}));
        }
        (s, job)
    }
    fn take(s: &AgentService, job: i64, request: &str) -> ServiceReply {
        let r = s.execute(
            "work_next",
            json!({"owner":"composite-test","job_id":job,"request_id":request}),
        );
        assert!(!r.is_error, "{}", r.metadata);
        r
    }
    fn assessment() -> Value {
        json!({"decision":"keep","style":"flat illustration","retained_elements":[],"reason":"Inspected final current pixels","model":"test"})
    }
    fn annotation() -> Value {
        json!({"caption":"A blue square with flat color.","reason":"Inspected final current pixels","model":"test"})
    }
    fn finish_input(work: &Value, request: &str) -> Value {
        let mut input = lease(work);
        input["expected_sha256"] = work["expected_sha256"].clone();
        input["request_id"] = json!(request);
        input["payload"] = json!({"assessment":assessment(),"annotation":annotation()});
        input
    }
    fn audit_count(f: &crate::workflow_tests::Fixture, action: &str) -> i64 {
        f.repo
            .connection
            .query_row(
                "SELECT count(*) FROM audit_log WHERE action=?1",
                [action],
                |r| r.get(0),
            )
            .unwrap()
    }
    #[test]
    fn next_claim_is_atomic_on_replay_and_does_not_claim_a_second_image() {
        let (f, _, m) = fixture();
        f.repo.connection.execute("INSERT INTO media(md5,metadata,keep) SELECT 'synthetic-next-image',metadata,0 FROM media WHERE id=?1",[m.id]).unwrap();
        let second = f.repo.connection.last_insert_rowid();
        f.repo
            .connection
            .execute(
                "UPDATE media SET metadata=json_set(metadata,'$.id',?1) WHERE id=?1",
                [second],
            )
            .unwrap();
        let (s, job) = ready(&f, vec![m.id, second], "annotate", json!(["caption"]));
        let first = take(&s, job, "next-1");
        assert!(first.image.is_some());
        let retry = take(&s, job, "next-1");
        assert!(retry.image.is_some());
        assert_eq!(first.metadata["work"], retry.metadata["work"]);
        assert_eq!(retry.metadata["claim"]["replayed"], true);
        let count: i64 = f
            .repo
            .connection
            .query_row(
                "SELECT count(*) FROM agent_work WHERE lease IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let next = take(&s, job, "next-2");
        assert_ne!(
            first.metadata["work"]["work_id"],
            next.metadata["work"]["work_id"]
        );
        let mismatch = s.execute(
            "work_next",
            json!({"owner":"other-owner","job_id":job,"request_id":"next-1"}),
        );
        assert_eq!(mismatch.metadata["code"], "idempotency_conflict");
        ok(&s, "work_release", lease(&first.metadata["work"]));
        let stale = s.execute(
            "work_next",
            json!({"owner":"composite-test","job_id":job,"request_id":"next-1"}),
        );
        assert!(stale.is_error);
        assert_eq!(stale.metadata["code"], "lease_lost");
    }
    #[test]
    fn empty_claim_receipt_never_silently_becomes_a_different_claim() {
        let (f, _, m) = fixture();
        let s = service(&f, true);
        let job=ok(&s,"job_submit",json!({"request_id":"unready","media_ids":[m.id],"mode":"annotate","outputs":["caption"]}))["job_id"].as_i64().unwrap();
        let empty = take(&s, job, "empty");
        assert!(empty.image.is_none());
        ok(&s, "jobs_tick", json!({}));
        let replay = take(&s, job, "empty");
        assert!(replay.image.is_none());
        assert!(replay.metadata["items"].as_array().unwrap().is_empty());
        assert!(take(&s, job, "ready").image.is_some());
    }

    #[test]
    fn concurrent_identical_next_requests_share_one_lease() {
        let (f, _, m) = fixture();
        let (s, job) = ready(&f, vec![m.id], "annotate", json!(["caption"]));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let s = s.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    take(&s, job, "concurrent").metadata["work"].clone()
                })
            })
            .collect();
        let rows: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(rows[0], rows[1]);
        let count: i64 = f
            .repo
            .connection
            .query_row("SELECT count(*) FROM agent_claims", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
    #[test]
    fn finish_replay_never_duplicates_assessment_or_annotation() {
        let (f, _, m) = fixture();
        let (s, job) = ready(&f, vec![m.id], "curate", json!(["caption"]));
        let work = take(&s, job, "claim").metadata["work"].clone();
        let input = finish_input(&work, "finish");
        let saved = ok(&s, "work_finish", input.clone());
        assert_eq!(saved["next_stage"], "completed");
        assert_eq!(saved["next"]["tool"], "bdm_work_next");
        assert_eq!(saved["assessment_saved"], true);
        assert_eq!(saved, ok(&s, "work_finish", input.clone()));
        assert_eq!(audit_count(&f, "agent_assessment"), 1);
        assert_eq!(audit_count(&f, "agent_annotation"), 1);
        let mut changed = input;
        changed["payload"]["annotation"]["caption"] = json!("A differently worded blue square.");
        assert_eq!(
            s.execute("work_finish", changed).metadata["code"],
            "idempotency_conflict"
        );
    }
    #[test]
    fn interrupted_finish_resumes_from_each_committed_child_without_rewriting() {
        for cut in [1, 2] {
            let (f, _, m) = fixture();
            let (s, job) = ready(&f, vec![m.id], "curate", json!(["caption"]));
            let work = take(&s, job, "claim").metadata["work"].clone();
            let input = json!(parse::<WorkFinishArgs>(finish_input(&work, "interrupted")).unwrap());
            let mut repo = s.connection(false).unwrap();
            assert!(begin_finish(&mut repo, &input).unwrap().is_none());
            ok(
                &s,
                "work_assess",
                child(&input, "assessment", input["payload"]["assessment"].clone()),
            );
            if cut == 2 {
                ok(
                    &s,
                    "work_annotate",
                    child(&input, "annotation", input["payload"]["annotation"].clone()),
                );
            }
            drop(repo);
            drop(s);
            let resumed = ok(&service(&f, true), "work_finish", input);
            assert_eq!(resumed["next_stage"], "completed");
            assert_eq!(audit_count(&f, "agent_assessment"), 1);
            assert_eq!(audit_count(&f, "agent_annotation"), 1);
        }
    }

    #[test]
    fn concurrent_finish_cannot_cache_a_transient_error_over_a_completed_write() {
        let (f, _, m) = fixture();
        let (s, job) = ready(&f, vec![m.id], "curate", json!(["caption"]));
        let work = take(&s, job, "claim").metadata["work"].clone();
        let input = finish_input(&work, "concurrent-finish");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let s = s.clone();
                let input = input.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    s.execute("work_finish", input)
                })
            })
            .collect();
        for handle in handles {
            let _ = handle.join().unwrap();
        }
        let replay = ok(&s, "work_finish", input);
        assert_eq!(replay["next_stage"], "completed");
        assert_eq!(audit_count(&f, "agent_assessment"), 1);
        assert_eq!(audit_count(&f, "agent_annotation"), 1);
    }
    #[test]
    fn finish_annotation_failure_preserves_assessment_and_allows_targeted_correction() {
        let (f, _, m) = fixture();
        let (s, job) = ready(&f, vec![m.id], "curate", json!(["caption"]));
        let work = take(&s, job, "claim").metadata["work"].clone();
        let before = std::fs::read(m.path.with_extension("txt")).ok();
        let mut input = finish_input(&work, "bad-caption");
        input["payload"]["annotation"]["caption"] = json!("");
        let rejected = s.execute("work_finish", input.clone());
        assert!(rejected.is_error);
        assert_eq!(rejected.metadata["assessment_saved"], true);
        assert_eq!(rejected.metadata["next_stage"], "annotation");
        assert_eq!(std::fs::read(m.path.with_extension("txt")).ok(), before);
        assert_eq!(rejected.metadata, s.execute("work_finish", input).metadata);
        let mut fix = lease(&work);
        fix["expected_sha256"] = work["expected_sha256"].clone();
        fix["request_id"] = json!("fix-caption");
        fix["payload"] = annotation();
        assert_eq!(ok(&s, "work_annotate", fix)["next_stage"], "completed");
        assert_eq!(audit_count(&f, "agent_assessment"), 1);
    }
    #[test]
    fn coverage_rejection_keeps_lease_and_rejects_all_fields_until_corrected() {
        let (f, _, mut m) = fixture();
        m.post.tag_string_general="scenery blue_hair smile long_sleeves hair_ornament soft_lighting looking_at_viewer shirt skirt blush hairclip".into();
        f.repo
            .connection
            .execute(
                "UPDATE media SET metadata=?1 WHERE id=?2",
                params![serde_json::to_string(&m).unwrap(), m.id],
            )
            .unwrap();
        let (s, job) = ready(&f, vec![m.id], "annotate", json!(["caption", "tags"]));
        let viewed = take(&s, job, "claim");
        let work = &viewed.metadata["work"];
        let before = std::fs::read(m.path.with_extension("txt")).ok();
        let mut input = lease(work);
        input["expected_sha256"] = work["expected_sha256"].clone();
        input["request_id"] = json!("thin-tags");
        input["payload"] = annotation();
        input["payload"]["tags"] = json!(["scenery"]);
        assert_eq!(
            s.execute("work_annotate", input.clone()).metadata["code"],
            "tag_coverage_incomplete"
        );
        assert_eq!(std::fs::read(m.path.with_extension("txt")).ok(), before);
        let state = ok(&s, "job_status", json!({"job_id":job}));
        assert_eq!(state["items"][0]["stage"], "annotation");
        input["request_id"] = json!("complete-tags");
        input["payload"]["tags"] = viewed.metadata["tag_vocabulary"]["baseline_tags"].clone();
        assert_eq!(ok(&s, "work_annotate", input)["next_stage"], "completed");
    }
    #[test]
    fn apply_and_view_returns_saved_png_pixels_and_allows_final_finish() {
        let (f, _, m) = fixture();
        let (s, job) = ready(&f, vec![m.id], "curate", json!(["caption"]));
        let work = take(&s, job, "claim").metadata["work"].clone();
        let mut proposed = assessment();
        proposed["crop_candidate"] = json!(true);
        ok(&s, "work_assess", leased_input(&work, "triage", proposed));
        let crop = json!({"ratio":"1:1","bbox_1000_xyxy":[20,20,980,980],"reason":"Retain the centered subject and trim margins","model":"test"});
        let preview = ok(
            &s,
            "work_preview",
            leased_input(&work, "preview", crop.clone()),
        );
        let mut apply = lease(&work);
        apply["expected_sha256"] = work["expected_sha256"].clone();
        apply["request_id"] = json!("apply-view");
        let mut exact = crop;
        exact["preview_sha256"] = preview["preview_sha256"].clone();
        apply["payload"] = json!({"crop":exact});
        let result = s.execute("work_apply_view", apply.clone());
        assert!(!result.is_error, "{}", result.metadata);
        assert!(result.image.is_some());
        assert_eq!(result.metadata["applied"], true);
        let current = f.repo.media_by_id(m.id).unwrap().unwrap();
        assert_eq!(current.path.extension().unwrap(), "png");
        assert_eq!(result.metadata["work"]["expected_sha256"], current.sha256);
        assert!(s.execute("work_apply_view", apply).image.is_some());
        assert_eq!(audit_count(&f, "agent_crop"), 1);
        let mut final_input = finish_input(&result.metadata["work"], "final");
        final_input["payload"]["assessment"]["crop_outcome"] = json!("applied");
        assert_eq!(
            ok(&s, "work_finish", final_input)["next_stage"],
            "completed"
        );
    }
    #[test]
    fn finish_cannot_skip_crop_preview_and_reject_never_saves_annotation_or_deletes_image() {
        let (f, _, m) = fixture();
        let (s, job) = ready(&f, vec![m.id], "curate", json!(["caption"]));
        let work = take(&s, job, "claim").metadata["work"].clone();
        let mut contradictory = finish_input(&work, "bad-reject");
        contradictory["payload"]["assessment"]["decision"] = json!("reject");
        assert_eq!(
            s.execute("work_finish", contradictory).metadata["code"],
            "invalid_finish"
        );
        assert_eq!(audit_count(&f, "agent_assessment"), 0);
        let mut proposed = assessment();
        proposed["crop_candidate"] = json!(true);
        ok(&s, "work_assess", leased_input(&work, "triage", proposed));
        let mut pending = finish_input(&work, "premature");
        pending["payload"]["assessment"]["crop_outcome"] = json!("not_improved");
        assert!(s.execute("work_finish", pending).is_error);
        assert_eq!(audit_count(&f, "agent_annotation"), 0);
        let crop = json!({"ratio":"1:1","bbox_1000_xyxy":[20,20,980,980],"reason":"Check composition before deciding","model":"test"});
        ok(&s, "work_preview", leased_input(&work, "preview", crop));
        let mut reject = finish_input(&work, "reject");
        reject["payload"]["assessment"]["crop_outcome"] = json!("not_improved");
        reject["payload"]["assessment"]["decision"] = json!("reject");
        reject["payload"]["annotation"] = Value::Null;
        assert_eq!(ok(&s, "work_finish", reject)["next_stage"], "review");
        assert!(m.path.exists());
        assert_eq!(audit_count(&f, "agent_annotation"), 0);
    }

    #[test]
    fn apply_view_reports_saved_crop_even_when_followup_view_loses_lease() {
        let (f, _, m) = fixture();
        let (s, job) = ready(&f, vec![m.id], "curate", json!(["caption"]));
        let work = take(&s, job, "claim").metadata["work"].clone();
        let mut proposed = assessment();
        proposed["crop_candidate"] = json!(true);
        ok(&s, "work_assess", leased_input(&work, "triage", proposed));
        let mut crop = json!({"ratio":"1:1","bbox_1000_xyxy":[20,20,980,980],"reason":"Preserve the centered composition","model":"test"});
        crop["preview_sha256"] = ok(
            &s,
            "work_preview",
            leased_input(&work, "preview", crop.clone()),
        )["preview_sha256"]
            .clone();
        ok(
            &s,
            "work_apply",
            leased_input(&work, "apply-interrupted", crop.clone()),
        );
        let saved = f.repo.media_by_id(m.id).unwrap().unwrap();
        ok(&s, "work_release", lease(&work));
        let mut input = lease(&work);
        input["expected_sha256"] = work["expected_sha256"].clone();
        input["request_id"] = json!("apply-interrupted");
        input["payload"] = json!({"crop":crop});
        let partial = s.execute("work_apply_view", input);
        assert!(partial.is_error);
        assert_eq!(partial.metadata["applied"], true);
        assert_eq!(partial.metadata["work"]["expected_sha256"], saved.sha256);
        assert_eq!(partial.metadata["code"], "lease_lost");
        assert!(partial.image.is_none());
        assert_eq!(
            f.repo.media_by_id(m.id).unwrap().unwrap().sha256,
            saved.sha256
        );
        assert_eq!(audit_count(&f, "agent_crop"), 1);
        assert!(take(&s, job, "resume-final-view").image.is_some());
    }
}
