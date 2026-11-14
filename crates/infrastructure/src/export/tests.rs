use super::*;

fn pixels(size: u32, color: u8, format: image::ImageFormat) -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        size,
        size,
        image::Rgb([color, 80, 130]),
    ))
    .write_to(&mut bytes, format)
    .unwrap();
    bytes.into_inner()
}

fn receipt(name: &str, bytes: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "schema":"bdm-export-v1", "media_id":1, "image":name,
        "sha256":format!("{:x}",Sha256::digest(bytes)),
        "source":{"provider":"danbooru","id":123,"md5":"source-original"},
        "text_kind":"caption", "text_sha256":"set-on-export"
    })
}

#[test]
fn flat_and_folder_refresh_images_preserve_text_bytes_and_reject_foreign_images() {
    for flat in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let old = pixels(8, 20, image::ImageFormat::Png);
        let new = pixels(16, 40, image::ImageFormat::Png);
        let manifest_path = if flat {
            root.join(".bdm-manifests/media_1.json")
        } else {
            root.join("manifest.json")
        };
        let save = |bytes: &[u8]| {
            let manifest = receipt("media_1.png", bytes);
            if flat {
                write_flat_pair(
                    root,
                    "media_1",
                    "media_1.png",
                    bytes,
                    "Generated text",
                    &manifest,
                )
            } else {
                write_managed_pair(
                    root,
                    "media_1",
                    "media_1.png",
                    bytes,
                    "Generated text",
                    &manifest,
                    &manifest_path,
                )
            }
        };
        save(&old).unwrap();
        let text = b"\xef\xbb\xbfUser text\r\n\xff";
        fs::write(root.join("media_1.txt"), text).unwrap();
        save(&new).unwrap();
        save(&new).unwrap();
        assert_eq!(fs::read(root.join("media_1.png")).unwrap(), new);
        assert_eq!(fs::read(root.join("media_1.txt")).unwrap(), text);
        let actual: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        assert_eq!(actual["text_sha256"], format!("{:x}", Sha256::digest(text)));
        assert_eq!(actual["text_preserved"], true);
        assert!(!root.join(".bdm-manifests/replaced-images").exists());
        assert!(!root
            .join(".bdm-manifests/media_1.image-update.json")
            .exists());
        fs::write(root.join("media_1.png"), b"Manual image").unwrap();
        assert!(matches!(save(&old), Err(IngestError::Integrity)));
        assert_eq!(fs::read(root.join("media_1.png")).unwrap(), b"Manual image");
        assert_eq!(fs::read(root.join("media_1.txt")).unwrap(), text);
    }
}

#[test]
fn jpeg_migration_removes_only_owned_images_and_matching_legacy_backups() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let old = pixels(8, 20, image::ImageFormat::Jpeg);
    let new = pixels(16, 40, image::ImageFormat::Png);
    write_flat_pair(
        root,
        "media_1",
        "media_1.jpg",
        &old,
        "Original TXT",
        &receipt("media_1.jpg", &old),
    )
    .unwrap();
    let backups = root.join(".bdm-manifests/replaced-images");
    fs::create_dir_all(&backups).unwrap();
    let backup = backups.join(format!("{:x}-media_1.jpg.bak", Sha256::digest(&old)));
    fs::write(&backup, &old).unwrap();
    let unrelated = backups.join("unrelated.bak");
    fs::write(&unrelated, b"User file").unwrap();
    write_flat_pair(
        root,
        "media_1",
        "media_1.png",
        &new,
        "New generated TXT",
        &receipt("media_1.png", &new),
    )
    .unwrap();
    assert!(!root.join("media_1.jpg").exists());
    assert!(!backup.exists());
    assert_eq!(fs::read(root.join("media_1.png")).unwrap(), new);
    assert_eq!(fs::read(root.join("media_1.txt")).unwrap(), b"Original TXT");
    assert_eq!(fs::read(unrelated).unwrap(), b"User file");
}

#[test]
fn interrupted_replacement_resumes_without_old_image_backups_or_text_writes() {
    for (ext, promoted, old_removed) in [
        ("png", false, false),
        ("png", true, false),
        ("jpg", true, true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let old_name = format!("media_1.{ext}");
        let old = pixels(
            8,
            20,
            if ext == "png" {
                image::ImageFormat::Png
            } else {
                image::ImageFormat::Jpeg
            },
        );
        let new = pixels(16, 40, image::ImageFormat::Png);
        write_flat_pair(
            root,
            "media_1",
            &old_name,
            &old,
            "Original TXT",
            &receipt(&old_name, &old),
        )
        .unwrap();
        let manifest_path = root.join(".bdm-manifests/media_1.json");
        let prior: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        let next = receipt("media_1.png", &new);
        let intent = root.join(".bdm-manifests/media_1.image-update.json");
        fs::write(
            &intent,
            serde_json::to_vec(&serde_json::json!({"previous":prior,"next":next})).unwrap(),
        )
        .unwrap();
        if promoted {
            fs::write(root.join("media_1.png"), &new).unwrap();
        }
        if old_removed {
            fs::remove_file(root.join(&old_name)).unwrap();
        }
        let text = b"User changed text during interruption\r\n";
        fs::write(root.join("media_1.txt"), text).unwrap();
        write_flat_pair(
            root,
            "media_1",
            "media_1.png",
            &new,
            "New generated TXT",
            &next,
        )
        .unwrap();
        assert_eq!(fs::read(root.join("media_1.png")).unwrap(), new);
        assert_eq!(fs::read(root.join("media_1.txt")).unwrap(), text);
        assert!(!intent.exists());
        assert!(!root.join(".bdm-manifests/replaced-images").exists());
        if ext != "png" {
            assert!(!root.join(old_name).exists());
        }
    }
}

#[test]
fn unowned_png_collision_and_changed_source_do_not_erase_existing_files() {
    for foreign_source in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let old = pixels(8, 20, image::ImageFormat::Jpeg);
        let new = pixels(16, 40, image::ImageFormat::Png);
        write_flat_pair(
            root,
            "media_1",
            "media_1.jpg",
            &old,
            "Original TXT",
            &receipt("media_1.jpg", &old),
        )
        .unwrap();
        let mut next = receipt("media_1.png", &new);
        if foreign_source {
            next["source"]["id"] = 999.into();
        } else {
            fs::write(root.join("media_1.png"), b"Manual image").unwrap();
        }
        assert!(matches!(
            write_flat_pair(root, "media_1", "media_1.png", &new, "New TXT", &next),
            Err(IngestError::Integrity)
        ));
        assert_eq!(fs::read(root.join("media_1.jpg")).unwrap(), old);
        assert_eq!(fs::read(root.join("media_1.txt")).unwrap(), b"Original TXT");
        if !foreign_source {
            assert_eq!(fs::read(root.join("media_1.png")).unwrap(), b"Manual image");
        }
    }
}
