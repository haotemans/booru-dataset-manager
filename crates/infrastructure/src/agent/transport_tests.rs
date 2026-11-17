use super::tests::{fixture, leased_input, ok, service, start_work};
use super::*;
use serde_json::Value;
use std::sync::Arc;

fn metadata(reply: CallToolResult) -> (Value, Value) {
    let raw = serde_json::to_value(reply).unwrap();
    assert_ne!(raw["isError"], true, "{raw}");
    let body = serde_json::from_str(raw["content"][0]["text"].as_str().unwrap()).unwrap();
    (body, raw)
}
fn envelope(item: &Value) -> Value {
    json!({"work_id":item["work_id"],"lease_token":item["lease_token"],"generation":item["generation"]})
}
fn resolved<'a>(root: &'a Value, node: &'a Value) -> &'a Value {
    if let Some(reference) = node["$ref"].as_str() {
        root.pointer(reference.strip_prefix('#').unwrap()).unwrap()
    } else {
        node
    }
}

#[test]
fn mcp_lists_complete_typed_payloads_and_keeps_legacy_entry() {
    let (f, _, _) = fixture();
    let server = AgentServer::open(
        &f.directory.path().join("catalog.sqlite3"),
        f.directory.path(),
        &f.directory.path().join("library"),
        false,
    )
    .unwrap();
    let tools = serde_json::to_value(server.tool_router.list_all()).unwrap();
    for name in [
        "bdm_agent",
        "bdm_job",
        "bdm_download",
        "bdm_work_claim",
        "bdm_work_view",
        "bdm_work_preview",
        "bdm_work_apply",
        "bdm_work_assess",
        "bdm_work_annotate",
        "bdm_work_control",
        "bdm_work_next",
        "bdm_work_apply_view",
        "bdm_work_finish",
    ] {
        let tool = tools
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap();
        assert_eq!(
            tool["inputSchema"]["type"], "object",
            "{name}: {}",
            tool["inputSchema"]
        );
    }
    for (name, forbidden) in [
        (
            "bdm_work_annotate",
            vec![
                "media_id",
                "expected_sha256",
                "overwrite",
                "require_final_assessment",
                "review_reasons",
            ],
        ),
        (
            "bdm_work_assess",
            vec!["media_id", "expected_sha256", "stage"],
        ),
        (
            "bdm_work_apply",
            vec!["media_id", "expected_sha256", "apply"],
        ),
    ] {
        let root = &tools
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap()["inputSchema"];
        let payload = resolved(root, &root["properties"]["payload"]);
        for field in forbidden {
            assert!(
                payload["properties"].get(field).is_none(),
                "{name} exposes {field}"
            );
        }
        assert_eq!(payload["additionalProperties"], false);
        assert!(payload["required"]
            .as_array()
            .unwrap()
            .contains(&json!("reason")));
        assert!(payload["required"]
            .as_array()
            .unwrap()
            .contains(&json!("model")));
    }
    let root = &tools
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "bdm_work_apply")
        .unwrap()["inputSchema"];
    let payload = resolved(root, &root["properties"]["payload"]);
    assert!(payload["required"]
        .as_array()
        .unwrap()
        .contains(&json!("preview_sha256")));
    let ratios = resolved(root, &payload["properties"]["ratio"])["enum"]
        .as_array()
        .unwrap();
    assert_eq!(
        ratios,
        &bdm_domain::training::RATIOS
            .iter()
            .map(|r| json!(r.0))
            .collect::<Vec<_>>()
    );
}

#[test]
fn typed_mcp_job_view_and_annotation_use_existing_durable_state() {
    let (f, _, m) = fixture();
    let server = AgentServer::open(
        &f.directory.path().join("catalog.sqlite3"),
        f.directory.path(),
        &f.directory.path().join("library"),
        true,
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async {
        let (submitted,_)=metadata(server.bdm_job(Parameters(serde_json::from_value(json!({"operation":"job_submit","input":{"request_id":"typed-job","mode":"annotate","media_ids":[m.id],"outputs":["caption"]}})).unwrap())).await);
        metadata(server.bdm_job(Parameters(JobRequest::Tick(EmptyArgs {}))).await);
        let (claimed,_)=metadata(server.bdm_work_claim(Parameters(serde_json::from_value(json!({"owner":"typed-test","job_id":submitted["job_id"],"limit":1})).unwrap())).await);
        assert_eq!(claimed["next"]["tool"],"bdm_work_view");
        let item=&claimed["items"][0];
        let (viewed,raw)=metadata(server.bdm_work_view(Parameters(serde_json::from_value(envelope(item)).unwrap())).await);
        assert_eq!(raw["content"][1]["mimeType"],"image/jpeg");
        assert_eq!(viewed["next"]["tool"],"bdm_work_annotate");
        let bytes=base64::Engine::decode(&base64::engine::general_purpose::STANDARD,raw["content"][1]["data"].as_str().unwrap()).unwrap();
        assert_eq!(image::guess_format(&bytes).unwrap(),image::ImageFormat::Jpeg);
        let mut input=envelope(item);
        input["expected_sha256"]=item["expected_sha256"].clone();
        input["request_id"]=json!("typed-caption");
        input["payload"]=json!({"caption":"A blue square with flat color.","reason":"Inspected the current image","model":"test"});
        let (saved,_)=metadata(server.bdm_work_annotate(Parameters(serde_json::from_value(input.clone()).unwrap())).await);
        assert_eq!(saved["next_stage"],"completed");
        let (replayed,_)=metadata(server.bdm_work_annotate(Parameters(serde_json::from_value(input.clone()).unwrap())).await);
        assert_eq!(saved,replayed);
        input["payload"]["overwrite"]=json!(true);
        assert!(serde_json::from_value::<WorkAnnotateArgs>(input).is_err());
        let state=ok(&service(&f,false),"job_status",json!({"job_id":submitted["job_id"]}));
        assert_eq!(state["counts"]["completed"],1);
    });
}

#[test]
fn preview_tokens_skip_only_an_already_inspected_full_view_in_this_lease() {
    let (f, _, m) = fixture();
    let original = std::fs::read(&m.path).unwrap();
    let (s, item, job) = start_work(&f, m.id, "annotate");
    let first = s.execute(
        "work_view",
        leased_input(
            &item,
            "v",
            json!({"preview":{"format":"jpeg","max_edge":768}}),
        ),
    );
    assert!(!first.is_error, "{}", first.metadata);
    assert!(first.image.is_some());
    let token = first.metadata["display_preview"]["token"].clone();
    let payload = json!({"preview":{"format":"jpeg","max_edge":768,"known_preview_token":token}});
    let second = s.execute("work_view", leased_input(&item, "v", payload.clone()));
    assert!(!second.is_error);
    assert!(second.image.is_none());
    assert_eq!(second.metadata["display_preview"]["cache_hit"], true);
    assert_eq!(second.metadata["display_preview"]["not_modified"], true);
    let other = s.execute(
        "work_view",
        leased_input(
            &item,
            "v",
            json!({"preview":{"format":"png","max_edge":768,"known_preview_token":token}}),
        ),
    );
    assert!(other.image.is_some());
    assert_ne!(other.metadata["display_preview"]["token"], token);
    ok(&s, "work_release", envelope(&item));
    let next = ok(&s, "work_claim", json!({"owner":"next-test","job_id":job}))["items"][0].clone();
    let fresh = s.execute("work_view", leased_input(&next, "v", payload));
    assert!(!fresh.is_error, "{}", fresh.metadata);
    assert!(
        fresh.image.is_some(),
        "New lease must inspect pixels even if preview is cached"
    );
    assert_eq!(fresh.metadata["display_preview"]["cache_hit"], true);
    assert_eq!(std::fs::read(&m.path).unwrap(), original);
    std::fs::write(&m.path, b"tampered file").unwrap();
    let stale = s.execute("work_view", leased_input(&next, "v", json!({})));
    assert!(stale.is_error);
    assert!(stale.image.is_none());
    assert_eq!(stale.metadata["code"], "stale_image");
}

#[test]
fn crop_preview_cache_keeps_native_png_proof_and_apply_requires_a_new_full_view() {
    let (mut f, settings, m) = fixture();
    let input = json!({"media_id":m.id,"expected_sha256":m.sha256,"bbox_1000_xyxy":[20,20,980,980],"ratio":"1:1","apply":false,"reason":"Preserve the centered subject","model":"test","preview":{"format":"jpeg","max_edge":768}});
    tools::view(
        &mut f.repo,
        &settings,
        serde_json::from_value(json!({"media_id":m.id})).unwrap(),
    )
    .unwrap();
    let first = tools::crop(
        &mut f.repo,
        &settings,
        serde_json::from_value(input.clone()).unwrap(),
    )
    .unwrap();
    let second = tools::crop(
        &mut f.repo,
        &settings,
        serde_json::from_value(input.clone()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        first.metadata["preview_sha256"],
        second.metadata["preview_sha256"]
    );
    assert_eq!(second.metadata["display_preview"]["cache_hit"], true);
    assert_eq!(second.image.as_ref().unwrap().mime_type, "image/jpeg");
    let mut apply = input;
    apply["apply"] = json!(true);
    apply["preview_sha256"] = first.metadata["preview_sha256"].clone();
    tools::crop(
        &mut f.repo,
        &settings,
        serde_json::from_value(apply).unwrap(),
    )
    .unwrap();
    let saved = f.repo.media_by_id(m.id).unwrap().unwrap();
    assert_eq!(saved.path.extension().unwrap(), "png");
    assert_eq!(saved.sha256, first.metadata["preview_sha256"]);
    let final_view=tools::view(&mut f.repo,&settings,serde_json::from_value(json!({"media_id":m.id,"preview":{"format":"jpeg","max_edge":768,"known_preview_token":first.metadata["display_preview"]["token"]}})).unwrap()).unwrap();
    assert!(final_view.image.is_some());
    assert_eq!(
        final_view.metadata["display_preview"]["not_modified"],
        false
    );
    assert_eq!(
        image::guess_format(&std::fs::read(saved.path).unwrap()).unwrap(),
        image::ImageFormat::Png
    );
}

#[test]
fn display_preview_is_bounded_and_does_not_upscale_transparent_pixels() {
    let (_f, settings, _) = fixture();
    let options = preview::PreviewOptions {
        format: preview::PreviewFormat::Jpeg,
        ..Default::default()
    };
    let result = preview::prepare(&settings, "transparent-native", &options, || {
        Ok((image::DynamicImage::new_rgba8(64, 32), Value::Null))
    })
    .unwrap()
    .reply(json!({}), &options, false);
    assert_eq!(result.metadata["display_dimensions"], json!([64, 32]));
    let pixels = image::load_from_memory(&result.image.unwrap().bytes)
        .unwrap()
        .to_rgb8();
    assert!(pixels.get_pixel(32, 16).0.iter().all(|c| *c >= 250));
    let invalid = preview::PreviewOptions {
        max_edge: 4096,
        ..Default::default()
    };
    assert!(preview::prepare(&settings, "invalid", &invalid, || panic!(
        "invalid options must not decode an image"
    ))
    .is_err());
    let hit = preview::prepare(&settings, "transparent-native", &options, || {
        panic!("cache hit must not decode or encode again")
    })
    .unwrap()
    .reply(json!({}), &options, false);
    assert_eq!(hit.metadata["display_preview"]["cache_hit"], true);
    assert!(Arc::strong_count(&hit.image.unwrap().bytes) >= 2);
}
