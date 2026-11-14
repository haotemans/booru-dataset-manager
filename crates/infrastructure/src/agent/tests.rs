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
        seen: Mutex::default(),
        previews: Mutex::default(),
    };
    (f, settings, m)
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
        style: "textured illustration".into(),
        decision,
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
    assert_eq!(tags, "@artist, character, special term, scenery");
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
    assert_eq!(server.tool_router.list_all().len(), 10);
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
