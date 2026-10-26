use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use bdm_config::JsonOutputMode;
use bdm_ports::IngestRepository;
use std::{
    io::{Read, Write},
    net::TcpListener,
};

fn profile() -> AiProfile {
    AiProfile {
        id: "synthetic-test".into(),
        endpoint: "http://127.0.0.1:1/v1".into(),
        model: "synthetic-model".into(),
        group: String::new(),
        credentials: None,
        settings: Default::default(),
        upload_approval: None,
        artist_tag_format: "legacy".into(),
        character_tag_format: "plain".into(),
    }
}

fn server(responses: Vec<(u16, Value)>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let thread = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, response) in responses {
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline, "request not received");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 8192];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(offset) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..offset]);
                    assert!(header.starts_with("POST /v1/chat/completions HTTP/1.1"));
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= offset + 4 + length {
                        requests.push(
                            serde_json::from_slice(&bytes[offset + 4..offset + 4 + length])
                                .unwrap(),
                        );
                        let response = response.to_string();
                        write!(stream, "HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
                        break;
                    }
                }
            }
        }
        requests
    });
    (endpoint, thread)
}

fn response(rows: Value) -> Value {
    json!({"choices":[{"message":{"content":json!({"schema":"bdm-text-v1","items":rows}).to_string()},"finish_reason":"stop"}]})
}

#[test]
fn image_compression_preserves_ratio_small_images_alpha_and_original_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    for (width, height, expected) in [
        (2048, 512, (1024, 256)),
        (512, 2048, (256, 1024)),
        (80, 40, (80, 40)),
        (1, 2000, (1, 1024)),
    ] {
        let path = directory.path().join(format!("{width}x{height}.png"));
        image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 0]))
            .save(&path)
            .unwrap();
        let original = std::fs::read(&path).unwrap();
        let prepared = images::prepare(1, &path, &AiRequestSettings::default(), &cancel).unwrap();
        let bytes = STANDARD
            .decode(
                prepared
                    .data_url
                    .strip_prefix("data:image/jpeg;base64,")
                    .unwrap(),
            )
            .unwrap();
        assert!(bytes.len() < 1024 * 1024);
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgb8();
        assert_eq!((decoded.width(), decoded.height()), expected);
        assert!(decoded.get_pixel(0, 0).0.iter().all(|c| *c >= 250));
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    let invalid = directory.path().join("bad.png");
    std::fs::write(&invalid, b"invalid image").unwrap();
    assert!(matches!(
        images::prepare(1, &invalid, &AiRequestSettings::default(), &cancel),
        Err(AiError::Image)
    ));
    assert!(matches!(
        images::prepare(
            1,
            &invalid,
            &AiRequestSettings::default(),
            &AtomicBool::new(true)
        ),
        Err(AiError::Cancelled)
    ));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 5);
}

#[test]
fn image_compression_adapts_quality_and_size_for_large_payloads() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("high-entropy.png");
    let image = image::RgbImage::from_fn(1024, 1024, |x, y| {
        image::Rgb([
            (x.wrapping_mul(37).wrapping_add(y * 13) as u8),
            (x.wrapping_mul(17).wrapping_add(y * 53) as u8),
            (x.wrapping_mul(91).wrapping_add(y * 7) as u8),
        ])
    });
    image.save(&path).unwrap();
    let mut settings = AiRequestSettings::default();
    settings.jpeg_quality = 95;
    let prepared = images::prepare(1, &path, &settings, &AtomicBool::new(false)).unwrap();
    let bytes = STANDARD
        .decode(
            prepared
                .data_url
                .strip_prefix("data:image/jpeg;base64,")
                .unwrap(),
        )
        .unwrap();
    assert!(bytes.len() <= 1024 * 1024);
    let decoded = image::load_from_memory(&bytes).unwrap();
    assert!(decoded.width() <= 1024 && decoded.height() <= 1024);
    assert!((decoded.width() as f32 / decoded.height() as f32 - 1.0).abs() < 0.01);
}

#[test]
fn strict_json_parser_preserves_valid_ids_and_rejects_invalid_results() {
    let expected = BTreeSet::from([1, 2, 3, 4]);
    assert!(matches!(
        protocol::parse("{}", &expected, "unknown"),
        Err(AiError::Configuration)
    ));
    let parsed = protocol::parse(
        &json!({"schema":"bdm-text-v1","items":[
            {"id":4,"caption":"Valid fourth result."},
            {"id":1,"caption":"First duplicate"},{"id":1,"caption":"Second duplicate"},
            {"id":2,"caption":3},{"id":99,"caption":"Unknown ID"},
            {"id":3,"caption":" ","extra":true}
        ]})
        .to_string(),
        &expected,
        "caption",
    )
    .unwrap();
    assert_eq!(parsed.items.keys().copied().collect::<Vec<_>>(), vec![4]);
    for content in [
        "```json\n{}\n```".into(),
        "{} trailing text".into(),
        json!({"schema":"wrong","items":[]}).to_string(),
        json!({"schema":"bdm-text-v1","items":{},"extra":true}).to_string(),
        "x".repeat(protocol::MAX_RESPONSE_BYTES + 1),
    ] {
        assert!(matches!(
            protocol::parse(&content, &expected, "caption"),
            Err(AiError::Content)
        ));
    }
    let parsed = protocol::parse(
        &json!({"schema":"bdm-text-v1","items":[
            {"id":1,"tags":["scenery"]},{"id":2,"tags":["bad\u{0000}tag"]},
            {"id":3,"tags":[42]},{"id":4,"tags":["a".repeat(257)]}
        ]})
        .to_string(),
        &expected,
        "tags",
    )
    .unwrap();
    assert_eq!(parsed.items.len(), 1);
}

#[test]
fn output_modes_are_explicit_and_multimodal_items_keep_ids() {
    let mut profile = profile();
    let input = vec![json!({"id":42,"tags":["scenery"]})];
    for mode in [
        JsonOutputMode::PromptOnly,
        JsonOutputMode::JsonObject,
        JsonOutputMode::JsonSchema,
    ] {
        profile.settings.json_output = mode;
        let body = protocol::body(&profile, &input, &[], "tags").unwrap();
        if mode == JsonOutputMode::PromptOnly {
            assert!(body.get("response_format").is_none());
        }
        if mode == JsonOutputMode::JsonSchema {
            assert_eq!(
                body["response_format"]["json_schema"]["schema"]["properties"]["items"]["maxItems"],
                1
            );
            assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        }
        assert!(body["messages"][1]["content"].is_string());
    }
    let images = vec![images::PreparedImage {
        id: 42,
        width: 80,
        height: 40,
        data_url: "data:image/jpeg;base64,c3ludGhldGlj".into(),
    }];
    let paired = vec![json!({"id":42,"tags":["scenery"],"paired_txt":"A scenic view."})];
    let body = protocol::body(&profile, &paired, &images, "caption").unwrap();
    let content = body["messages"][1]["content"].as_array().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(content[1]["text"].as_str().unwrap()).unwrap()
            ["image_item_id"],
        42
    );
    let tag_body = protocol::body(&profile, &paired, &images, "tags").unwrap();
    assert!(tag_body["messages"][0]["content"]
        .as_str().unwrap().contains(&profile.settings.tag_system_prompt));
    assert!(body["messages"][0]["content"]
        .as_str().unwrap().contains(&profile.settings.caption_system_prompt));
    assert!(protocol::body(&profile, &input, &images, "tags").is_err());
    assert!(protocol::body(&profile, &[paired[0].clone(), paired[0].clone()], &images, "caption").is_err());
}

#[test]
fn caption_upload_requires_scope_approval_and_sends_compressed_json_to_local_service() {
    let mut f = crate::workflow_tests::fixture();
    let cancel = AtomicBool::new(false);
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            f.collection,
            800,
            &cancel,
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    let original = std::fs::read(&media.path).unwrap();
    let mut profile = profile();
    profile.settings.send_images = true;
    profile.settings.image_max_edge = 512;
    let mut job = f.repo.new_workflow("optimize", vec![media.id]).unwrap();
    job.mode = "caption".into();
    run(&mut f.repo, &mut job, &profile, &cancel, |_| {}).unwrap();
    assert_eq!(job.state, "awaiting_approval");
    assert!(job.accepted.is_empty());
    let (endpoint, server) = server(vec![(
        200,
        response(json!([{"id":media.id,"caption":"A landscape."}])),
    )]);
    profile.endpoint = endpoint;
    profile.upload_approval = Some(UploadApproval {
        profile_id: profile.id.clone(),
        endpoint: profile.endpoint.clone(),
        model: profile.model.clone(),
        settings: profile.settings.clone(),
        ids: BTreeSet::from([media.id]),
    });
    run(&mut f.repo, &mut job, &profile, &cancel, |_| {}).unwrap();
    let requests = server.join().unwrap();
    let content = requests[0]["messages"][1]["content"].as_array().unwrap();
    let input: Value = serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(input["items"][0]["id"], media.id);
    let bytes = STANDARD
        .decode(
            content[2]["image_url"]["url"]
                .as_str()
                .unwrap()
                .strip_prefix("data:image/jpeg;base64,")
                .unwrap(),
        )
        .unwrap();
    let decoded = image::load_from_memory(&bytes).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (512, 512));
    assert_eq!(job.state, "completed");
    assert_eq!(std::fs::read(&media.path).unwrap(), original);
    let artifact = f
        .repo
        .list_artifacts()
        .unwrap()
        .into_iter()
        .find(|a| a.kind == "caption")
        .unwrap();
    assert_eq!(artifact.state, "needs_review");
    assert!(artifact.content.starts_with("@artist artist, character, "));
}

#[test]
fn client_configuration_errors_pause_without_failing_items() {
    let mut f = crate::workflow_tests::fixture();
    let cancel = AtomicBool::new(false);
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            f.collection,
            800,
            &cancel,
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    let (endpoint, server) = server(vec![(400, json!({"error":{"code":"model_not_found"}}))]);
    let mut profile = profile();
    profile.endpoint = endpoint;
    let mut job = f.repo.new_workflow("optimize", vec![media.id]).unwrap();
    job.mode = "tags".into();
    run(&mut f.repo, &mut job, &profile, &cancel, |_| {}).unwrap();
    assert_eq!(server.join().unwrap().len(), 1);
    assert_eq!(job.state, "paused");
    assert_eq!(job.cursor, 0);
    assert!(job.deferred.is_empty());
    assert!(job.error.as_ref().unwrap().contains("API"));
    assert!(job.analysis_errors.contains_key(&media.id));
}

#[test]
fn content_filter_is_persisted_as_failure_and_successful_resume_clears_it() {
    let mut f = crate::workflow_tests::fixture();
    let cancel = AtomicBool::new(false);
    f.repo
        .import(
            &f.post,
            &f.directory.path().join("library"),
            f.collection,
            800,
            &cancel,
            &f.provider,
        )
        .unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    let caption_before = std::fs::read(media.path.with_extension("txt")).ok();
    let blocked = json!({"choices":[{"finish_reason":"content_filter","message":{"content":""}}]});
    let (endpoint, worker) = server(vec![(200, blocked.clone()), (200, blocked)]);
    let mut profile = profile();
    profile.endpoint = endpoint;
    profile.settings.batch_size = 1;
    let mut job = f.repo.new_workflow("optimize", vec![media.id]).unwrap();
    job.mode = "caption".into();
    run(&mut f.repo, &mut job, &profile, &cancel, |_| {}).unwrap();
    assert_eq!(worker.join().unwrap().len(), 2);
    assert_eq!(job.state, "deferred");
    assert!(job.accepted.is_empty());
    assert_eq!(job.deferred, vec![media.id]);
    assert!(job.error.as_ref().unwrap().contains("content_filter"));
    assert_eq!(job.analysis_errors[&media.id].len(), 2);
    assert_eq!(
        std::fs::read(media.path.with_extension("txt")).ok(),
        caption_before
    );
    assert!(!f
        .repo
        .list_artifacts()
        .unwrap()
        .iter()
        .any(|a| a.kind == "caption"));
    let persisted = f
        .repo
        .list_workflows()
        .unwrap()
        .into_iter()
        .find(|j| j.id == job.id)
        .unwrap();
    assert_eq!(persisted.analysis_errors, job.analysis_errors);
    let count: i64 = f
        .repo
        .connection
        .query_row(
            "SELECT count(*) FROM audit_log WHERE action='api_batch_failure'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);

    let (endpoint, worker) = server(vec![(
        200,
        response(json!([{"id":media.id,"caption":"A landscape."}])),
    )]);
    profile.endpoint = endpoint;
    run(&mut f.repo, &mut job, &profile, &cancel, |_| {}).unwrap();
    assert_eq!(worker.join().unwrap().len(), 1);
    assert_eq!(job.state, "completed");
    assert!(job.accepted.contains(&media.id));
    assert!(job.deferred.is_empty());
    assert!(job.analysis_errors.is_empty());
    assert!(job.error.is_none());
}

#[test]
fn explicit_api_refusal_does_not_log_remote_text() {
    let (endpoint, worker) = server(vec![(
        200,
        json!({"choices":[{"finish_reason":"stop","message":{"content":null,"refusal":"remote-sensitive-detail"}}]}),
    )]);
    let mut profile = profile();
    profile.endpoint = endpoint;
    let error = request(
        &Client::new(),
        &profile,
        &[json!({"id":1,"tags":[]})],
        &[],
        "caption",
        &AtomicBool::new(false),
    )
    .err()
    .unwrap();
    assert!(matches!(error, AiError::ContentFiltered));
    assert!(!error.message().contains("remote-sensitive-detail"));
    assert_eq!(worker.join().unwrap().len(), 1);
}

#[test]
fn vision_tagging_sends_one_image_and_its_txt_and_requires_review() {
    let mut f = crate::workflow_tests::fixture();
    let cancel = AtomicBool::new(false);
    f.repo.import(
        &f.post, &f.directory.path().join("library"), f.collection, 800, &cancel, &f.provider
    ).unwrap();
    let media = f.repo.list_media().unwrap().remove(0);
    let txt = media.path.with_extension("txt");
    std::fs::write(&txt, "scenery, blue sky").unwrap();
    let (endpoint, worker) = server(vec![(
        200, response(json!([{"id":media.id,"tags":["scenery","blue_sky"]}]))
    )]);
    let mut profile = profile();
    profile.endpoint = endpoint;
    profile.settings.send_images = true;
    profile.settings.batch_size = 8;
    profile.upload_approval = Some(UploadApproval {
        profile_id: profile.id.clone(),
        endpoint: profile.endpoint.clone(),
        model: profile.model.clone(),
        settings: profile.settings.clone(),
        ids: BTreeSet::from([media.id])
    });
    let mut job = f.repo.new_workflow("optimize", vec![media.id]).unwrap();
    job.mode = "tags".into();
    run(&mut f.repo, &mut job, &profile, &cancel, |_| {}).unwrap();
    let requests = worker.join().unwrap();
    assert_eq!(requests.len(), 1);
    let content = requests[0]["messages"][1]["content"].as_array().unwrap();
    assert_eq!(content.len(), 3);
    let input: Value = serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(input["items"][0]["id"], media.id);
    assert_eq!(input["items"][0]["paired_txt"], "scenery, blue sky");
    let artifact = f.repo.list_artifacts().unwrap().into_iter().find(|a| a.kind=="tags").unwrap();
    assert_eq!(artifact.state, "needs_review");
    assert!(artifact.content.contains("blue_sky"));
    assert_eq!(std::fs::read_to_string(txt).unwrap(), "scenery, blue sky");
    assert_eq!(job.state, "completed");
}
