use crate::{catalog::SqliteCatalog, ingest::storage};
use bdm_domain::{
    editing::{CropRect, EditRecord, MaskRegion},
    ingest::StoredMedia,
};
use bdm_ports::{IngestError, MediaProvider};
use image::{GenericImageView, ImageFormat};
use rusqlite::params;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

const WORKER: &str = include_str!("../../../scripts/inpainting_worker.py");
mod blend;
mod mask;
mod worker;

pub fn model_directory(root: &Path) -> PathBuf {
    root.join("models/inpainting")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdm_ports::IngestRepository;
    fn imported() -> crate::workflow_tests::Fixture {
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
        f
    }
    fn synthetic(media: &StoredMedia) -> RepairPreview {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("preview.png");
        let mask = directory.path().join("mask.png");
        let mut image = image::open(&media.path).unwrap().to_rgba8();
        for y in 100..120 {
            for x in 100..140 {
                image.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        image.save(&output).unwrap();
        image::GrayImage::from_fn(image.width(), image.height(), |x, y| {
            image::Luma([if (100..140).contains(&x) && (100..120).contains(&y) {
                255
            } else {
                0
            }])
        })
        .save(&mask)
        .unwrap();
        RepairPreview {
            directory,
            image: output,
            mask,
            parent_sha256: media.sha256.clone(),
            model: "synthetic-test".into(),
            regions: vec![MaskRegion::Rectangle {
                start: [100.0, 100.0],
                end: [140.0, 120.0],
            }],
            crop: None,
            training: None,
        }
    }
    #[test]
    fn training_crop_keeps_download_bytes_roundtrips_restore_and_exports_matching_config() {
        let mut f = crate::workflow_tests::fixture();
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2000,
            1600,
            image::Rgb([30, 80, 150]),
        ))
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
        f.provider.bytes = encoded.into_inner();
        f.post.image_width = 2000;
        f.post.image_height = 1600;
        f.post.file_size = f.provider.bytes.len() as u64;
        f.post.md5 = format!("{:x}", md5::Md5::digest(&f.provider.bytes));
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
        let original = fs::read(&media.path).unwrap();
        let raw = fs::read(media.path.with_extension("raw_tags.txt")).unwrap();
        let preview = training_crop_preview(
            &media,
            CropRect {
                x: 100,
                y: 100,
                width: 1600,
                height: 1280,
            },
            1024,
            &cancel,
        )
        .unwrap();
        assert_eq!(
            image::open(&preview.image).unwrap().dimensions(),
            (1120, 896)
        );
        let first = f.repo.commit_repair(media.id, &preview).unwrap();
        let archive = first
            .edit
            .as_ref()
            .unwrap()
            .source_archive
            .as_ref()
            .unwrap();
        assert!(archive.downloaded_original);
        assert_eq!(fs::read(&archive.path).unwrap(), original);
        assert_eq!(first.post, media.post);
        assert_eq!(
            fs::read(first.path.with_extension("raw_tags.txt")).unwrap(),
            raw
        );
        let preview = training_crop_preview(
            &first,
            CropRect {
                x: 0,
                y: 0,
                width: 1120,
                height: 896,
            },
            768,
            &cancel,
        )
        .unwrap();
        let second = f.repo.commit_repair(media.id, &preview).unwrap();
        assert_eq!(
            second.edit.as_ref().unwrap().source_archive.as_ref(),
            Some(archive)
        );
        let restored = f.repo.restore_last_edit(media.id).unwrap();
        assert_eq!(restored.dimensions(), first.dimensions());
        assert_eq!(restored.edit, first.edit);
        assert_eq!(hash(&restored.path).unwrap(), first.sha256);
        assert_eq!(fs::read(&archive.path).unwrap(), original);
        f.repo.set_media_keep(media.id, true).unwrap();
        f.repo
            .write_artifact(
                &restored,
                "caption",
                "A blue training image.",
                "test",
                "complete",
            )
            .unwrap();
        let mut job = f.repo.new_workflow("export", vec![media.id]).unwrap();
        job.mode = "caption".into();
        job.export_layout = "flat".into();
        let destination = f.directory.path().join("training-export");
        crate::export::run(&mut f.repo, &mut job, &destination, &cancel, |_| {}).unwrap();
        let config = fs::read_to_string(destination.join("dataset.toml")).unwrap();
        assert!(config.contains("resolution = [1024, 1024]"));
        assert!(config.contains("bucket_no_upscale = true"));
        assert!(config.contains("bucket_reso_steps = 32"));
        assert_eq!(
            image::image_dimensions(destination.join(format!("media_{}.png", media.id))).unwrap(),
            (1120, 896)
        );
        let manual = format!("{config}\n# user modification\n");
        fs::write(destination.join("dataset.toml"), &manual).unwrap();
        let mut repeated = f.repo.new_workflow("export", vec![media.id]).unwrap();
        repeated.mode = "caption".into();
        repeated.export_layout = "flat".into();
        assert!(
            crate::export::run(&mut f.repo, &mut repeated, &destination, &cancel, |_| {}).is_err()
        );
        assert_eq!(
            fs::read_to_string(destination.join("dataset.toml")).unwrap(),
            manual
        );
    }

    #[test]
    fn redownload_restores_cropped_and_repaired_originals_with_backup_and_source_metadata() {
        for crop in [false, true] {
            let mut f = imported();
            let media = f.repo.list_media().unwrap().remove(0);
            let original = fs::read(&media.path).unwrap();
            let raw_tags = fs::read(media.path.with_extension("raw_tags.txt")).unwrap();
            let preview = if crop {
                crop_preview(
                    &media,
                    CropRect {
                        x: 5,
                        y: 6,
                        width: 300,
                        height: 200,
                    },
                    &AtomicBool::new(false),
                )
                .unwrap()
            } else {
                synthetic(&media)
            };
            let edited = f.repo.commit_repair(media.id, &preview).unwrap();
            let edited_bytes = fs::read(&edited.path).unwrap();
            assert_ne!(edited_bytes, original);
            let restored = f
                .repo
                .redownload_original(media.id, &f.provider, &AtomicBool::new(false))
                .unwrap();
            assert_eq!(restored, media);
            assert_eq!(f.repo.list_media().unwrap()[0], restored);
            assert_eq!(fs::read(&restored.path).unwrap(), original);
            assert_eq!(
                image::open(&restored.path).unwrap().dimensions(),
                restored.dimensions()
            );
            assert_eq!(
                fs::read(restored.path.with_extension("raw_tags.txt")).unwrap(),
                raw_tags
            );
            let backups: Vec<_> = fs::read_dir(restored.path.parent().unwrap())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(".redownload-backup-")
                })
                .collect();
            assert_eq!(backups.len(), 1);
            assert_eq!(fs::read(&backups[0]).unwrap(), edited_bytes);
            f.repo.recover_repairs().unwrap();
            assert_eq!(f.repo.list_media().unwrap()[0], restored);
        }
    }

    #[test]
    fn redownload_rejects_invalid_source_without_replacing_edited_file() {
        let mut f = imported();
        let media = f.repo.list_media().unwrap().remove(0);
        let edited = f.repo.commit_repair(media.id, &synthetic(&media)).unwrap();
        let bytes = fs::read(&edited.path).unwrap();
        let thumbnail = fs::read(&edited.thumbnail).unwrap();
        f.provider.bytes[0] ^= 1;
        assert!(matches!(
            f.repo
                .redownload_original(media.id, &f.provider, &AtomicBool::new(false)),
            Err(IngestError::Integrity)
        ));
        assert_eq!(f.repo.list_media().unwrap()[0], edited);
        assert_eq!(fs::read(&edited.path).unwrap(), bytes);
        assert_eq!(fs::read(&edited.thumbnail).unwrap(), thumbnail);
    }
    #[test]
    fn crop_is_local_exact_and_repeated_edits_export_current_dimensions() {
        let mut f = imported();
        let media = f.repo.list_media().unwrap().remove(0);
        let original = image::open(&media.path).unwrap();
        let cancel = AtomicBool::new(false);
        let rect = CropRect {
            x: 50,
            y: 60,
            width: 600,
            height: 400,
        };
        assert!(crop_preview(&media, rect, &AtomicBool::new(true)).is_err());
        assert!(crop_preview(
            &media,
            CropRect {
                x: u32::MAX,
                ..rect
            },
            &cancel
        )
        .is_err());
        let preview = crop_preview(&media, rect, &cancel).unwrap();
        assert_eq!(
            image::open(&preview.image).unwrap().to_rgba8(),
            original.crop_imm(50, 60, 600, 400).to_rgba8()
        );
        assert_eq!(hash(&media.path).unwrap(), media.sha256);
        let edited = f.repo.commit_repair(media.id, &preview).unwrap();
        assert_eq!(edited.dimensions(), (600, 400));
        assert_eq!(edited.post, media.post);
        assert_eq!(edited.id, media.id);
        assert_eq!(edited.edit.as_ref().unwrap().crop, Some(rect));
        assert!(edited.edit.as_ref().unwrap().mask.as_os_str().is_empty());
        assert!(f.repo.commit_repair(media.id, &preview).is_err());
        let repair = synthetic(&edited);
        let repaired = f.repo.commit_repair(media.id, &repair).unwrap();
        assert_eq!(repaired.dimensions(), (600, 400));
        let preview = crop_preview(
            &repaired,
            CropRect {
                x: 5,
                y: 6,
                width: 300,
                height: 200,
            },
            &cancel,
        )
        .unwrap();
        let cropped = f.repo.commit_repair(media.id, &preview).unwrap();
        assert_eq!(cropped.dimensions(), (300, 200));
        assert_eq!(cropped.edit.as_ref().unwrap().source_sha256, media.sha256);
        f.repo.set_media_keep(media.id, true).unwrap();
        f.repo
            .write_artifact(&cropped, "caption", "A cropped scene.", "test", "complete")
            .unwrap();
        let mut job = f.repo.new_workflow("export", vec![media.id]).unwrap();
        job.mode = "caption".into();
        let destination = f.directory.path().join("export");
        crate::export::run(&mut f.repo, &mut job, &destination, &cancel, |_| {}).unwrap();
        let directory = destination.join(format!("media_{}", media.id));
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(
            manifest["current_dimensions"],
            serde_json::json!([300, 200])
        );
        assert_eq!(manifest["source"]["image_width"], 800);
    }
    #[test]
    fn crop_recovery_finishes_confirmed_file_and_dimensions_after_database_failure() {
        let mut f = imported();
        let media = f.repo.list_media().unwrap().remove(0);
        let preview = crop_preview(
            &media,
            CropRect {
                x: 20,
                y: 30,
                width: 300,
                height: 500,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        f.repo.connection.execute_batch("CREATE TRIGGER fail_crop BEFORE UPDATE OF metadata ON media BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(f.repo.commit_repair(media.id, &preview).is_err());
        f.repo
            .connection
            .execute_batch("DROP TRIGGER fail_crop;")
            .unwrap();
        f.repo.recover_repairs().unwrap();
        let edited = f.repo.list_media().unwrap().remove(0);
        assert_eq!(edited.dimensions(), (300, 500));
        assert_eq!(
            image::open(&edited.path).unwrap().dimensions(),
            edited.dimensions()
        );
        assert_eq!(hash(&edited.path).unwrap(), edited.sha256);
        assert_eq!(edited.post, media.post);
    }
    #[test]
    fn cancellation_stale_preview_and_confirmed_edit_preserve_source_text_and_identity() {
        let mut f = imported();
        let media = f.repo.list_media().unwrap().remove(0);
        let raw = fs::read(media.path.with_extension("raw_tags.txt")).unwrap();
        let tags = f.repo.list_artifacts().unwrap()[0].content.clone();
        let original = fs::read(&media.path).unwrap();
        let cancelled = synthetic(&media);
        let temp = cancelled.directory.path().to_owned();
        drop(cancelled);
        assert!(!temp.exists());
        assert_eq!(fs::read(&media.path).unwrap(), original);
        let mut stale = synthetic(&media);
        stale.parent_sha256 = "stale".into();
        assert!(f.repo.commit_repair(media.id, &stale).is_err());
        assert_eq!(fs::read(&media.path).unwrap(), original);
        let preview = synthetic(&media);
        let edited = f.repo.commit_repair(media.id, &preview).unwrap();
        assert_eq!(edited.path, media.path);
        assert_eq!(edited.post, media.post);
        assert_eq!(edited.collections, media.collections);
        assert_ne!(edited.sha256, media.sha256);
        assert_eq!(edited.edit.as_ref().unwrap().source_sha256, media.sha256);
        assert_eq!(hash(&edited.path).unwrap(), edited.sha256);
        assert_eq!(
            fs::read(media.path.with_extension("raw_tags.txt")).unwrap(),
            raw
        );
        assert_eq!(f.repo.list_artifacts().unwrap()[0].content, tags);
        assert_eq!(f.repo.list_media().unwrap().len(), 1);
        f.repo.recover_repairs().unwrap();
        let preview = synthetic(&edited);
        let next = f.repo.commit_repair(media.id, &preview).unwrap();
        assert_eq!(next.edit.unwrap().source_sha256, media.sha256);
        assert!(!media
            .path
            .parent()
            .unwrap()
            .join(".repair-journal.json")
            .exists());
    }
    #[test]
    fn edited_jpeg_webp_and_png_save_lossless_png_with_matching_paths_and_exports() {
        for (extension, format) in [
            ("jpg", ImageFormat::Jpeg),
            ("webp", ImageFormat::WebP),
            ("png", ImageFormat::Png),
        ] {
            let mut f = crate::workflow_tests::fixture();
            let image = image::load_from_memory(&f.provider.bytes)
                .unwrap()
                .to_rgb8();
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image)
                .write_to(&mut bytes, format)
                .unwrap();
            f.provider.bytes = bytes.into_inner();
            f.post.md5 = format!("{:x}", md5::Md5::digest(&f.provider.bytes));
            f.post.file_ext = extension.into();
            f.post.file_size = f.provider.bytes.len() as u64;
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
            let media = f.repo.list_media().unwrap().remove(0);
            let original_bytes = fs::read(&media.path).unwrap();
            let preview = synthetic(&media);
            let edited = f.repo.commit_repair(media.id, &preview).unwrap();
            assert_eq!(
                image::guess_format(&fs::read(&edited.path).unwrap()).unwrap(),
                ImageFormat::Png
            );
            assert_eq!(edited.current_extension(), "png");
            assert_eq!(edited.post, media.post);
            assert_eq!(
                image::open(&edited.path).unwrap().to_rgba8(),
                image::open(&preview.image).unwrap().to_rgba8()
            );
            let restored = f.repo.restore_last_edit(media.id).unwrap();
            assert_eq!(restored.path, media.path);
            assert_eq!(fs::read(&restored.path).unwrap(), original_bytes);
            let preview = synthetic(&restored);
            let edited = f.repo.commit_repair(media.id, &preview).unwrap();
            assert_eq!(image::open(&edited.path).unwrap().dimensions(), (800, 800));
            let cropped = crop_preview(
                &edited,
                CropRect {
                    x: 50,
                    y: 60,
                    width: 600,
                    height: 400,
                },
                &AtomicBool::new(false),
            )
            .unwrap();
            let edited = f.repo.commit_repair(media.id, &cropped).unwrap();
            assert_eq!(
                image::guess_format(&fs::read(&edited.path).unwrap()).unwrap(),
                ImageFormat::Png
            );
            assert_eq!(image::open(&edited.path).unwrap().dimensions(), (600, 400));
            assert_eq!(
                image::open(&edited.path).unwrap().to_rgba8(),
                image::open(&cropped.image).unwrap().to_rgba8()
            );
            f.repo.set_media_keep(edited.id, true).unwrap();
            f.repo
                .write_artifact(&edited, "caption", "A test image.", "test", "complete")
                .unwrap();
            let mut job = f.repo.new_workflow("export", vec![edited.id]).unwrap();
            job.mode = "caption".into();
            job.export_layout = "flat".into();
            let output = f.directory.path().join("png-export");
            crate::export::run(
                &mut f.repo,
                &mut job,
                &output,
                &AtomicBool::new(false),
                |_| {},
            )
            .unwrap();
            assert_eq!(
                fs::read(output.join(format!("media_{}.png", edited.id))).unwrap(),
                fs::read(&edited.path).unwrap()
            );
            assert!(!output.join(format!("media_{}.jpg", edited.id)).exists());
            let downloaded = f
                .repo
                .redownload_original(edited.id, &f.provider, &AtomicBool::new(false))
                .unwrap();
            assert_eq!(downloaded.path, media.path);
            assert_eq!(fs::read(downloaded.path).unwrap(), original_bytes);
        }
    }

    #[test]
    fn legacy_jpeg_crop_rebuild_uses_verified_original_and_safely_migrates_flat_export() {
        let mut f = crate::workflow_tests::fixture();
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::load_from_memory(&f.provider.bytes)
            .unwrap()
            .write_to(&mut bytes, ImageFormat::Jpeg)
            .unwrap();
        f.provider.bytes = bytes.into_inner();
        f.post.file_ext = "jpg".into();
        f.post.file_size = f.provider.bytes.len() as u64;
        f.post.md5 = format!("{:x}", md5::Md5::digest(&f.provider.bytes));
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
        let original = f.repo.list_media().unwrap().remove(0);
        let crop = CropRect {
            x: 16,
            y: 16,
            width: 768,
            height: 768,
        };
        let preview = training_crop_preview(&original, crop, 2048, &cancel).unwrap();
        let mut legacy = f.repo.commit_repair(original.id, &preview).unwrap();
        let png_path = legacy.path.clone();
        let mut lossy = std::io::Cursor::new(Vec::new());
        image::open(&png_path)
            .unwrap()
            .write_to(&mut lossy, ImageFormat::Jpeg)
            .unwrap();
        fs::write(&original.path, lossy.get_ref()).unwrap();
        fs::remove_file(&png_path).unwrap();
        legacy.path = original.path.clone();
        legacy.sha256 = hash(&legacy.path).unwrap();
        legacy.edit.as_mut().unwrap().output_sha256 = legacy.sha256.clone();
        f.repo
            .connection
            .execute(
                "UPDATE media SET metadata=?2 WHERE id=?1",
                params![legacy.id, serde_json::to_string(&legacy).unwrap()],
            )
            .unwrap();
        f.repo.set_media_keep(legacy.id, true).unwrap();
        f.repo
            .write_artifact(&legacy, "caption", "A test crop.", "test", "complete")
            .unwrap();
        let export = f.directory.path().join("export");
        let mut job = f.repo.new_workflow("export", vec![legacy.id]).unwrap();
        job.mode = "caption".into();
        job.export_layout = "flat".into();
        crate::export::run(&mut f.repo, &mut job, &export, &cancel, |_| {}).unwrap();
        let backup = f.directory.path().join("repair-backup");
        assert_eq!(
            f.repo.repair_jpeg_crops_as_png(&backup, false).unwrap()[0].state,
            "ready"
        );
        assert_eq!(hash(&legacy.path).unwrap(), legacy.sha256);
        assert_eq!(
            f.repo.repair_jpeg_crops_as_png(&backup, true).unwrap()[0].state,
            "rebuilt"
        );
        let rebuilt = f.repo.list_media().unwrap().remove(0);
        assert_eq!(rebuilt.current_extension(), "png");
        assert!(rebuilt.keep);
        assert_eq!(rebuilt.post, original.post);
        assert_eq!(
            image::open(&rebuilt.path).unwrap().to_rgba8(),
            image::open(&preview.image).unwrap().to_rgba8()
        );
        assert_eq!(
            fs::read(backup.join(format!("media_{}/before.jpg", legacy.id))).unwrap(),
            lossy.into_inner()
        );
        assert!(backup.join("catalog-before.sqlite3").is_file());
        job.cursor = 0;
        crate::export::run(&mut f.repo, &mut job, &export, &cancel, |_| {}).unwrap();
        assert!(!export.join(format!("media_{}.jpg", legacy.id)).exists());
        assert_eq!(
            fs::read(export.join(format!("media_{}.png", legacy.id))).unwrap(),
            fs::read(&rebuilt.path).unwrap()
        );
        assert_eq!(
            fs::read_dir(export.join(".bdm-manifests/replaced-images"))
                .unwrap()
                .count(),
            1
        );
        assert!(f
            .repo
            .repair_jpeg_crops_as_png(&backup, false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn png_path_transition_recovers_after_catalog_failure_without_touching_source_jpeg() {
        let mut f = crate::workflow_tests::fixture();
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::load_from_memory(&f.provider.bytes)
            .unwrap()
            .write_to(&mut encoded, ImageFormat::Jpeg)
            .unwrap();
        f.provider.bytes = encoded.into_inner();
        f.post.file_ext = "jpg".into();
        f.post.md5 = format!("{:x}", md5::Md5::digest(&f.provider.bytes));
        f.post.file_size = f.provider.bytes.len() as u64;
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
        let media = f.repo.list_media().unwrap().remove(0);
        let preview = synthetic(&media);
        f.repo.connection.execute_batch("CREATE TRIGGER fail_png BEFORE UPDATE OF metadata ON media BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(f.repo.commit_repair(media.id, &preview).is_err());
        assert_eq!(hash(&media.path).unwrap(), media.sha256);
        f.repo
            .connection
            .execute_batch("DROP TRIGGER fail_png;")
            .unwrap();
        f.repo.recover_repairs().unwrap();
        let edited = f.repo.list_media().unwrap().remove(0);
        assert_eq!(edited.current_extension(), "png");
        assert_eq!(
            image::open(&edited.path).unwrap().to_rgba8(),
            image::open(&preview.image).unwrap().to_rgba8()
        );
        assert_eq!(hash(&media.path).unwrap(), media.sha256);
    }

    #[test]
    fn confirmed_edit_recovers_after_file_replacement_before_catalog_commit() {
        let mut f = imported();
        let media = f.repo.list_media().unwrap().remove(0);
        let preview = synthetic(&media);
        f.repo.connection.execute_batch("CREATE TRIGGER fail_edit BEFORE UPDATE OF metadata ON media BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(f.repo.commit_repair(media.id, &preview).is_err());
        assert!(media
            .path
            .parent()
            .unwrap()
            .join(".repair-journal.json")
            .is_file());
        f.repo
            .connection
            .execute_batch("DROP TRIGGER fail_edit;")
            .unwrap();
        f.repo.recover_repairs().unwrap();
        let edited = f.repo.list_media().unwrap().remove(0);
        assert_eq!(hash(&edited.path).unwrap(), edited.sha256);
        assert!(edited.edit.is_some());
        assert!(!media
            .path
            .parent()
            .unwrap()
            .join(".repair-journal.json")
            .exists());
    }
    #[test]
    #[ignore = "Runs installed Anime LaMa locally on a synthetic image; no image upload"]
    fn native_anime_lama_preview_preserves_unmasked_pixels_and_current_file() {
        let f = imported();
        let media = f.repo.list_media().unwrap().remove(0);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let bytes = fs::read(&media.path).unwrap();
        let started = std::time::Instant::now();
        let preview = preview(
            &root,
            &media,
            "anime-lama",
            vec![MaskRegion::Rectangle {
                start: [100.0, 100.0],
                end: [150.0, 140.0],
            }],
            &AtomicBool::new(false),
        )
        .unwrap();
        let output = image::open(&preview.image).unwrap().to_rgba8();
        let original = image::open(&media.path).unwrap().to_rgba8();
        assert_eq!(output.dimensions(), original.dimensions());
        assert_eq!(fs::read(&media.path).unwrap(), bytes);
        for (x, y, p) in original.enumerate_pixels() {
            if !(100..150).contains(&x) || !(100..140).contains(&y) {
                assert_eq!(output.get_pixel(x, y), p);
            }
        }
        assert!(original
            .enumerate_pixels()
            .any(|(x, y, p)| output.get_pixel(x, y) != p));
        println!(
            "Real Anime LaMa preview completed in {:?}",
            started.elapsed()
        );
        let started = std::time::Instant::now();
        let brush = MaskRegion::Brush {
            points: (0..240)
                .map(|i| {
                    let angle = i as f32 * std::f32::consts::TAU / 239.0;
                    [630.0 + 40.0 * angle.cos(), 640.0 + 55.0 * angle.sin()]
                })
                .collect(),
            radius: 2.0,
        };
        let preview = super::preview(
            &root,
            &media,
            "anime-lama",
            vec![brush.clone()],
            &AtomicBool::new(false),
        )
        .unwrap();
        let output = image::open(&preview.image).unwrap().to_rgba8();
        assert_eq!(fs::read(&media.path).unwrap(), bytes);
        for (x, y, p) in original.enumerate_pixels() {
            if !brush.contains(x as f32 + 0.5, y as f32 + 0.5) {
                assert_eq!(output.get_pixel(x, y), p);
            }
        }
        assert!(original
            .enumerate_pixels()
            .any(|(x, y, p)| output.get_pixel(x, y) != p));
        println!(
            "Real Anime LaMa long-brush preview completed in {:?}",
            started.elapsed()
        );
    }

    #[test]
    #[ignore = "Measures full-resolution brush mask generation on synthetic pixels"]
    fn long_brush_mask_benchmark() {
        let regions = [MaskRegion::Brush {
            points: (0..240)
                .map(|i| {
                    let angle = i as f32 * std::f32::consts::TAU / 239.0;
                    [820.0 + 85.0 * angle.cos(), 800.0 + 110.0 * angle.sin()]
                })
                .collect(),
            radius: 2.0,
        }];
        let cancel = AtomicBool::new(false);
        for size in [(1024, 1024), (4000, 3000)] {
            let started = std::time::Instant::now();
            let selected = mask::rasterize(size.0, size.1, &regions, &cancel).unwrap();
            println!("Bounded brush mask {size:?}: {:?}", started.elapsed());
            assert!(selected.pixels().any(|p| p[0] > 0));
        }
    }
}
pub fn runtime(root: &Path) -> PathBuf {
    root.join(if cfg!(windows) {
        "target/analysis-runtime/Scripts/python.exe"
    } else {
        "target/analysis-runtime/bin/python"
    })
}
pub fn model_ready(root: &Path, model: &str) -> bool {
    runtime(root).is_file() && model_directory(root).join(format!("{model}.pt")).is_file()
}
pub fn hash(path: &Path) -> Result<String, IngestError> {
    Ok(format!("{:x}", Sha256::digest(storage(fs::read(path))?)))
}

fn run(mut command: Command, cancel: &AtomicBool) -> Result<(), IngestError> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = storage(command.stdout(Stdio::null()).stderr(Stdio::piped()).spawn())?;
    let error = child.stderr.take().ok_or(IngestError::Storage)?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut message = String::new();
        let _ = error.take(8192).read_to_string(&mut message);
        let _ = sender.send(message);
    });
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(IngestError::Cancelled);
        }
        if let Some(status) = storage(child.try_wait())? {
            if status.success() {
                return Ok(());
            }
            let message = receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap_or_default();
            return Err(IngestError::StorageDetails(
                message
                    .chars()
                    .rev()
                    .take(900)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect(),
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn install(root: &Path, model: &str, cancel: &AtomicBool) -> Result<(), IngestError> {
    if !matches!(model, "anime-lama" | "lama") {
        return Err(IngestError::InvalidMedia);
    }
    let mut command = Command::new(runtime(root));
    command
        .args(["-c", WORKER, "--model", model, "--model-dir"])
        .arg(model_directory(root))
        .arg("--download");
    run(command, cancel)
}

pub struct RepairPreview {
    pub directory: tempfile::TempDir,
    pub image: PathBuf,
    pub mask: PathBuf,
    pub parent_sha256: String,
    pub model: String,
    pub regions: Vec<MaskRegion>,
    pub crop: Option<CropRect>,
    pub training: Option<bdm_domain::training::TrainingCrop>,
}

pub fn crop_preview(
    media: &StoredMedia,
    crop: CropRect,
    cancel: &AtomicBool,
) -> Result<RepairPreview, IngestError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(IngestError::StorageDetails("任务已取消。".into()));
    }
    let parent_sha256 = hash(&media.path)?;
    if parent_sha256 != media.sha256 {
        return Err(IngestError::Integrity);
    }
    let decoded = storage(image::open(&media.path))?;
    if decoded.dimensions() != media.dimensions() || !crop.valid(decoded.dimensions()) {
        return Err(IngestError::InvalidMedia);
    }
    let directory = storage(tempfile::Builder::new().prefix("bdm-crop-").tempdir())?;
    let output = directory.path().join("preview.png");
    storage(
        decoded
            .crop_imm(crop.x, crop.y, crop.width, crop.height)
            .save(&output),
    )?;
    if cancel.load(Ordering::Relaxed) {
        return Err(IngestError::StorageDetails("任务已取消。".into()));
    }
    Ok(RepairPreview {
        directory,
        image: output,
        mask: PathBuf::new(),
        parent_sha256,
        model: "local-crop-v1".into(),
        regions: Vec::new(),
        crop: Some(crop),
        training: None,
    })
}

pub fn training_crop_preview(
    media: &StoredMedia,
    crop: CropRect,
    resolution: u32,
    cancel: &AtomicBool,
) -> Result<RepairPreview, IngestError> {
    let plan = bdm_domain::training::plan(crop.width, crop.height, resolution)
        .map_err(|error| IngestError::StorageDetails(error.into()))?;
    let mut preview = crop_preview(media, crop, cancel)?;
    let image = storage(image::open(&preview.image))?;
    let resized = if image.dimensions() == (plan.resized[0], plan.resized[1]) {
        image
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
    storage(output.save(&preview.image))?;
    if cancel.load(Ordering::Relaxed) {
        return Err(IngestError::Cancelled);
    }
    preview.model = "local-training-crop-kohya32-v1".into();
    preview.training = Some(plan);
    Ok(preview)
}

pub fn preview(
    root: &Path,
    media: &StoredMedia,
    model: &str,
    regions: Vec<MaskRegion>,
    cancel: &AtomicBool,
) -> Result<RepairPreview, IngestError> {
    if !matches!(model, "anime-lama" | "lama") || !model_ready(root, model) {
        return Err(IngestError::StorageDetails(
            "请先安装所选 LaMa 模型。".into(),
        ));
    }
    let parent_sha256 = hash(&media.path)?;
    if parent_sha256 != media.sha256 {
        return Err(IngestError::Integrity);
    }
    let decoded = storage(image::open(&media.path))?;
    let (width, height) = decoded.dimensions();
    let mask_image = mask::rasterize(width, height, &regions, cancel)?;
    if !mask_image.pixels().any(|p| p[0] > 0) {
        return Err(IngestError::InvalidMedia);
    }
    let directory = storage(tempfile::Builder::new().prefix("bdm-inpaint-").tempdir())?;
    let mask = directory.path().join("mask.png");
    let output = directory.path().join("preview.png");
    storage(mask_image.save(&mask))?;
    worker::infer(root, model, &media.path, &mask, &output, cancel)?;
    let result = storage(image::open(&output))?;
    if result.dimensions() != (width, height) {
        return Err(IngestError::Integrity);
    }
    // Match each repair to its clean border and preserve unselected pixels/alpha.
    let mut composed = decoded.to_rgba8();
    let repair = result.to_rgba8();
    if !repair_quality_ok(&composed, &repair, &mask_image) {
        return Err(IngestError::StorageDetails(
            "模型预览出现异常噪声，已阻止替换原图；请缩小选区或切换修复模型后重试。".into(),
        ));
    }
    blend::compose(&mut composed, &repair, &mask_image, cancel)?;
    storage(composed.save(&output))?;
    Ok(RepairPreview {
        directory,
        image: output,
        mask,
        parent_sha256,
        model: if model == "anime-lama" {
            "anime-lama@b592884b2b6589e57df6916db6bcac7a6307fc58:sha256-95d4bf7eb2f13729351ac0a0693c9b9b469b264fc1542ca3fa551b59b464a160:crop128-max1024-v1:interior-tone-feather16-v4".into()
        } else {
            "lama:md5-e3aa4aaa15225a33ec84f9f4bc47e500:crop128-max1024-v1:interior-tone-feather16-v4"
                .into()
        },
        regions,
        crop: None,
        training: None,
    })
}

fn repair_quality_ok(
    original: &image::RgbaImage,
    repaired: &image::RgbaImage,
    mask: &image::GrayImage,
) -> bool {
    let mut original_gradient = 0u64;
    let mut repaired_gradient = 0u64;
    let mut samples = 0u64;
    for y in 1..original.height() {
        for x in 1..original.width() {
            if mask.get_pixel(x, y)[0] == 0 {
                continue;
            }
            let left = mask.get_pixel(x - 1, y)[0] > 0;
            let above = mask.get_pixel(x, y - 1)[0] > 0;
            if !left && !above {
                continue;
            }
            for channel in 0..3 {
                let original_pixel = original.get_pixel(x, y)[channel] as i32;
                let repaired_pixel = repaired.get_pixel(x, y)[channel] as i32;
                if left {
                    original_gradient += (original_pixel
                        - original.get_pixel(x - 1, y)[channel] as i32)
                        .unsigned_abs() as u64;
                    repaired_gradient += (repaired_pixel
                        - repaired.get_pixel(x - 1, y)[channel] as i32)
                        .unsigned_abs() as u64;
                }
                if above {
                    original_gradient += (original_pixel
                        - original.get_pixel(x, y - 1)[channel] as i32)
                        .unsigned_abs() as u64;
                    repaired_gradient += (repaired_pixel
                        - repaired.get_pixel(x, y - 1)[channel] as i32)
                        .unsigned_abs() as u64;
                }
                samples += (if left { 1 } else { 0 }) + (if above { 1 } else { 0 });
            }
        }
    }
    if samples < 32 {
        return true;
    }
    let original_average = original_gradient as f32 / samples as f32;
    let repaired_average = repaired_gradient as f32 / samples as f32;
    repaired_average <= original_average * 3.5 + 35.0
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Journal {
    media: StoredMedia,
    image: PathBuf,
    thumbnail: PathBuf,
    mask: PathBuf,
    source: PathBuf,
    #[serde(default)]
    parent: Option<StoredMedia>,
}

fn staged(path: &Path, bytes: &[u8]) -> Result<(), IngestError> {
    let mut file = storage(fs::File::create(path))?;
    storage(file.write_all(bytes))?;
    storage(file.sync_all())
}

fn preserve_source(
    media: &StoredMedia,
    directory: &Path,
) -> Result<bdm_domain::editing::SourceArchive, IngestError> {
    if let Some(archive) = media
        .edit
        .as_ref()
        .and_then(|edit| edit.source_archive.clone())
    {
        if archive.path.parent() != Some(directory) || hash(&archive.path)? != archive.sha256 {
            return Err(IngestError::Integrity);
        }
        return Ok(archive);
    }
    let downloaded = directory.join(format!("downloaded-original.{}", media.post.file_ext));
    let bytes = storage(fs::read(if downloaded.is_file() {
        &downloaded
    } else {
        &media.path
    }))?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let downloaded_original = format!("{:x}", md5::Md5::digest(&bytes)) == media.post.md5;
    if downloaded.is_file() {
        if !downloaded_original {
            return Err(IngestError::Integrity);
        }
        return Ok(bdm_domain::editing::SourceArchive {
            path: downloaded,
            sha256,
            downloaded_original: true,
        });
    }
    let path = directory.join(format!(
        ".original-preserved-{sha256}.{}",
        media.current_extension()
    ));
    if path.is_file() {
        if hash(&path)? != sha256 {
            return Err(IngestError::Integrity);
        }
    } else {
        let mut file = storage(tempfile::NamedTempFile::new_in(directory))?;
        storage(file.write_all(&bytes))?;
        storage(file.as_file().sync_all())?;
        storage(file.persist_noclobber(&path).map_err(|error| error.error))?;
    }
    Ok(bdm_domain::editing::SourceArchive {
        path,
        sha256,
        downloaded_original,
    })
}

fn validate_png_destination(media: &StoredMedia, parent: &StoredMedia) -> Result<(), IngestError> {
    if media.path == parent.path || !media.path.exists() || hash(&media.path)? == media.sha256 {
        return Ok(());
    }
    let receipt = media
        .path
        .parent()
        .ok_or(IngestError::Storage)?
        .join(".png-output.json");
    if receipt.is_file() {
        let owned: serde_json::Value =
            storage(serde_json::from_slice(&storage(fs::read(receipt))?))?;
        if owned["media_id"] == media.id
            && owned["path"] == serde_json::json!(media.path)
            && owned["sha256"].as_str() == Some(hash(&media.path)?.as_str())
        {
            return Ok(());
        }
    }
    Err(IngestError::StorageDetails(
        "同名 PNG 已存在且不属于已确认的编辑结果，已保留文件并停止保存。".into(),
    ))
}

#[derive(Debug, serde::Serialize)]
pub struct CropPngRepair {
    pub media_id: i64,
    pub state: String,
    pub reason: String,
}

pub fn write_crop_png_report(path: &Path, report: &[CropPngRepair]) -> Result<(), IngestError> {
    staged(path, &storage(serde_json::to_vec_pretty(report))?)
}

fn verified_crop_parent(media: &StoredMedia) -> Result<StoredMedia, IngestError> {
    let edit = media.edit.as_ref().ok_or(IngestError::InvalidMedia)?;
    let crop = edit.crop.ok_or(IngestError::InvalidMedia)?;
    let directory = media.path.parent().ok_or(IngestError::Storage)?;
    let archive = edit
        .source_archive
        .as_ref()
        .ok_or_else(|| IngestError::StorageDetails("缺少原始输入存档，未自动修改。".into()))?;
    let parent_bytes = storage(fs::read(directory.join(".edit-parent.json")))?;
    let parent: StoredMedia = storage(serde_json::from_slice(&parent_bytes))?;
    if !archive.downloaded_original
        || archive.path.parent() != Some(directory)
        || parent.id != media.id
        || parent.post != media.post
        || parent.edit.is_some()
        || parent.path != media.path
        || parent.sha256 != edit.parent_sha256
        || hash(&directory.join(".edit-source"))? != parent.sha256
        || archive.sha256 != parent.sha256
        || hash(&archive.path)? != parent.sha256
        || format!("{:x}", md5::Md5::digest(storage(fs::read(&archive.path))?)) != media.post.md5
        || hash(&media.path)? != media.sha256
        || !crop.valid(parent.dimensions())
    {
        return Err(IngestError::Integrity);
    }
    let expected = if let Some(plan) = &edit.training {
        let verified = bdm_domain::training::plan(crop.width, crop.height, plan.resolution)
            .map_err(|_| IngestError::Integrity)?;
        if &verified != plan {
            return Err(IngestError::Integrity);
        }
        (plan.output[0], plan.output[1])
    } else {
        (crop.width, crop.height)
    };
    if expected != media.dimensions() || media.path.with_extension("png").exists() {
        return Err(IngestError::Integrity);
    }
    Ok(parent)
}

impl SqliteCatalog {
    /// Offline, explicitly invoked remediation. Rebuild only a provable direct
    /// crop of the downloaded original, preserving its selection and dimensions.
    pub fn repair_jpeg_crops_as_png(
        &mut self,
        backup: &Path,
        apply: bool,
    ) -> Result<Vec<CropPngRepair>, IngestError> {
        storage(fs::create_dir_all(backup))?;
        if apply && !backup.join("catalog-before.sqlite3").exists() {
            storage(self.connection.backup(
                rusqlite::DatabaseName::Main,
                backup.join("catalog-before.sqlite3"),
                None,
            ))?;
        }
        let mut report = Vec::new();
        for media in self.list_media()? {
            if !matches!(media.current_extension(), "jpg" | "jpeg")
                || !media
                    .edit
                    .as_ref()
                    .is_some_and(|edit| edit.operation == "crop")
            {
                continue;
            }
            let result = (|| -> Result<(), IngestError> {
                let mut source = verified_crop_parent(&media)?;
                if !apply {
                    return Ok(());
                }
                let edit = media.edit.as_ref().ok_or(IngestError::InvalidMedia)?;
                source.path = edit
                    .source_archive
                    .as_ref()
                    .ok_or(IngestError::Integrity)?
                    .path
                    .clone();
                let crop = edit.crop.ok_or(IngestError::Integrity)?;
                let cancel = AtomicBool::new(false);
                let mut preview = if let Some(plan) = &edit.training {
                    training_crop_preview(&source, crop, plan.resolution, &cancel)?
                } else {
                    crop_preview(&source, crop, &cancel)?
                };
                preview.model = "local-crop-png-rebuilt-v2".into();
                let saved = backup.join(format!("media_{}", media.id));
                storage(fs::create_dir_all(&saved))?;
                let previous = saved.join("before.jpg");
                if previous.exists() {
                    if hash(&previous)? != media.sha256 {
                        return Err(IngestError::Integrity);
                    }
                } else {
                    storage(fs::copy(&media.path, &previous))?;
                }
                staged(
                    &saved.join("metadata-before.json"),
                    &storage(serde_json::to_vec(&media))?,
                )?;
                let directory = media.path.parent().ok_or(IngestError::Storage)?;
                storage(fs::copy(
                    directory.join(".edit-parent.json"),
                    saved.join("parent-before.json"),
                ))?;
                let restored = self.restore_last_edit(media.id)?;
                if restored.sha256 != preview.parent_sha256 {
                    return Err(IngestError::Integrity);
                }
                let rebuilt = self.commit_repair(media.id, &preview)?;
                if rebuilt.dimensions() != media.dimensions()
                    || storage(image::open(&rebuilt.path))?.to_rgba8()
                        != storage(image::open(&preview.image))?.to_rgba8()
                {
                    return Err(IngestError::Integrity);
                }
                Ok(())
            })();
            report.push(CropPngRepair {
                media_id: media.id,
                state: if result.is_ok() {
                    if apply {
                        "rebuilt"
                    } else {
                        "ready"
                    }
                } else {
                    "unchanged"
                }
                .into(),
                reason: result
                    .err()
                    .map_or_else(String::new, |error| error.to_string()),
            });
        }
        Ok(report)
    }

    pub fn redownload_original(
        &mut self,
        id: i64,
        provider: &dyn MediaProvider,
        cancel: &AtomicBool,
    ) -> Result<StoredMedia, IngestError> {
        let media = self.media_by_id(id)?.ok_or(IngestError::InvalidMedia)?;
        let directory = media.path.parent().ok_or(IngestError::Storage)?;
        if directory.join(".repair-journal.json").exists() {
            return Err(IngestError::StorageDetails(
                "当前图片有未完成的保存任务，请重启程序恢复后重试。".into(),
            ));
        }
        let staging = storage(
            tempfile::Builder::new()
                .prefix(".redownload-")
                .tempdir_in(directory),
        )?;
        let downloaded = staging
            .path()
            .join(format!("original.{}", media.post.file_ext));
        provider.download(&media.post, &downloaded, cancel)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        let bytes = storage(fs::read(&downloaded))?;
        if (media.post.file_size != 0 && bytes.len() as u64 != media.post.file_size)
            || bytes.is_empty()
            || bytes.len() > 64 * 1024 * 1024
        {
            return Err(IngestError::InvalidMedia);
        }
        if format!("{:x}", md5::Md5::digest(&bytes)) != media.post.md5 {
            return Err(IngestError::Integrity);
        }
        let decoded = crate::ingest::validate(&bytes, &media.post, media.minimum_short_side)?;
        let source_sha256 = format!("{:x}", Sha256::digest(&bytes));
        let (stored_image, stored_bytes) =
            if media.flatten_transparency && crate::ingest::has_transparency(&decoded) {
                let flattened = crate::ingest::flatten_on_white(&decoded);
                let format = match media.post.file_ext.as_str() {
                    "png" => ImageFormat::Png,
                    "webp" => ImageFormat::WebP,
                    _ => ImageFormat::Jpeg,
                };
                let mut encoded = std::io::Cursor::new(Vec::new());
                flattened
                    .write_to(&mut encoded, format)
                    .map_err(|_| IngestError::InvalidMedia)?;
                (flattened, encoded.into_inner())
            } else {
                (decoded, bytes)
            };
        let staged_image = staging.path().join("replacement");
        storage(fs::write(&staged_image, &stored_bytes))?;
        let staged_thumbnail = staging.path().join("preview.png");
        storage(
            stored_image
                .thumbnail(384, 384)
                .save_with_format(&staged_thumbnail, ImageFormat::Png),
        )?;
        let backup = directory.join(format!(
            ".redownload-backup-{}-{}",
            media.sha256.get(..16).unwrap_or("current"),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_secs())
        ));
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        let current = self.media_by_id(id)?.ok_or(IngestError::InvalidMedia)?;
        if current.sha256 != media.sha256
            || current.edit != media.edit
            || directory.join(".repair-journal.json").exists()
        {
            return Err(IngestError::StorageDetails(
                "图片在下载期间发生变化，请重新执行下载。".into(),
            ));
        }
        if media.path.is_file() {
            storage(fs::copy(&media.path, &backup))?;
        }
        let replacement = storage(tempfile::NamedTempFile::new_in(directory))?;
        storage(fs::copy(&staged_image, replacement.path()))?;
        let restored_path = media.path.with_extension(&media.post.file_ext);
        storage(replacement.persist(&restored_path).map_err(|e| e.error))?;
        let replacement_thumbnail = storage(tempfile::NamedTempFile::new_in(directory))?;
        storage(fs::copy(&staged_thumbnail, replacement_thumbnail.path()))?;
        storage(
            replacement_thumbnail
                .persist(&media.thumbnail)
                .map_err(|e| e.error),
        )?;

        // Keep review/collection changes made during the transfer, while restoring source dimensions.
        let mut updated = current;
        updated.path = restored_path;
        updated.edit = None;
        updated.sha256 = format!("{:x}", Sha256::digest(&stored_bytes));
        updated.source_sha256 = source_sha256;
        let tx = storage(
            self.connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
        )?;
        storage(tx.execute(
            "UPDATE media SET metadata=?2 WHERE id=?1",
            params![updated.id, storage(serde_json::to_string(&updated))?],
        ))?;
        for table in ["visual_features", "visual_buckets", "media_quality"] {
            storage(tx.execute(
                &format!("DELETE FROM {table} WHERE media_id=?1"),
                [updated.id],
            ))?;
        }
        storage(tx.execute(
            "DELETE FROM duplicate_candidates WHERE first_id=?1 OR second_id=?1",
            [updated.id],
        ))?;
        storage(tx.execute(
            "UPDATE analysis_jobs SET state='pending',reason='original redownloaded' WHERE media_id=?1",
            [updated.id],
        ))?;
        storage(tx.execute(
            "INSERT INTO audit_log(action,metadata) VALUES('original_redownload',?1)",
            [format!(
                "media_id={};backup={}",
                updated.id,
                backup.display()
            )],
        ))?;
        storage(tx.commit())?;
        Ok(updated)
    }

    pub fn commit_repair(
        &mut self,
        id: i64,
        preview: &RepairPreview,
    ) -> Result<StoredMedia, IngestError> {
        self.commit_repair_cancellable(id, preview, &AtomicBool::new(false))
    }

    pub fn commit_repair_cancellable(
        &mut self,
        id: i64,
        preview: &RepairPreview,
        cancel: &AtomicBool,
    ) -> Result<StoredMedia, IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::StorageDetails("任务已取消。".into()));
        }
        let mut media = self.media_by_id(id)?.ok_or(IngestError::InvalidMedia)?;
        let parent = media.clone();
        if media.sha256 != preview.parent_sha256 || hash(&media.path)? != preview.parent_sha256 {
            return Err(IngestError::Integrity);
        }
        let directory = media.path.parent().ok_or(IngestError::Storage)?.to_owned();
        let image = storage(image::open(&preview.image))?;
        if let Some(crop) = preview.crop {
            let expected = if let Some(plan) = &preview.training {
                let verified = bdm_domain::training::plan(crop.width, crop.height, plan.resolution)
                    .map_err(|_| IngestError::Integrity)?;
                if plan != &verified {
                    return Err(IngestError::Integrity);
                }
                (plan.output[0], plan.output[1])
            } else {
                (crop.width, crop.height)
            };
            if !crop.valid(media.dimensions()) || image.dimensions() != expected {
                return Err(IngestError::Integrity);
            }
        } else {
            if image.dimensions() != media.dimensions() {
                return Err(IngestError::Integrity);
            }
            let selected = storage(image::open(&preview.mask))?.to_luma8();
            if selected.dimensions() != image.dimensions() || !selected.pixels().any(|p| p[0] > 0) {
                return Err(IngestError::Integrity);
            }
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        storage(image.write_to(&mut bytes, ImageFormat::Png))?;
        let bytes = bytes.into_inner();
        storage(image::load_from_memory(&bytes))?;
        let pending = directory.join(".repair-output");
        let thumbnail = directory.join(".repair-thumbnail");
        let source = directory.join(".edit-source");
        let mask = if preview.crop.is_some() {
            PathBuf::new()
        } else {
            directory.join(".repair-mask")
        };
        staged(&pending, &bytes)?;
        staged(&source, &storage(fs::read(&media.path))?)?;
        let mut thumb_bytes = std::io::Cursor::new(Vec::new());
        storage(
            image
                .thumbnail(320, 320)
                .write_to(&mut thumb_bytes, ImageFormat::Png),
        )?;
        staged(&thumbnail, thumb_bytes.get_ref())?;
        if preview.crop.is_none() {
            staged(&mask, &storage(fs::read(&preview.mask))?)?;
        }
        let source_sha256 = media
            .edit
            .as_ref()
            .map_or_else(|| media.sha256.clone(), |e| e.source_sha256.clone());
        let source_archive = preserve_source(&media, &directory)?;
        media.path = media.path.with_extension("png");
        media.sha256 = format!("{:x}", Sha256::digest(&bytes));
        let training = preview.training.clone().or_else(|| {
            if preview.crop.is_none() {
                media.edit.as_ref().and_then(|edit| edit.training.clone())
            } else {
                None
            }
        });
        media.edit = Some(EditRecord {
            operation: if preview.crop.is_some() {
                "crop"
            } else {
                "inpainting"
            }
            .into(),
            model: preview.model.clone(),
            source_sha256,
            parent_sha256: preview.parent_sha256.clone(),
            output_sha256: media.sha256.clone(),
            mask: if preview.crop.is_some() {
                PathBuf::new()
            } else {
                directory.join("inpainting-mask.png")
            },
            regions: preview.regions.clone(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            dimensions: Some([image.width(), image.height()]),
            crop: preview.crop,
            training,
            source_archive: Some(source_archive),
        });
        validate_png_destination(&media, &parent)?;
        let journal = Journal {
            media,
            image: pending,
            thumbnail,
            mask,
            source,
            parent: Some(parent),
        };
        // Cancellation is safe before durable intent. Once published, finish the journaled edit.
        if cancel.load(Ordering::Relaxed) {
            for path in [
                &journal.image,
                &journal.thumbnail,
                &journal.mask,
                &journal.source,
            ] {
                if !path.as_os_str().is_empty() {
                    storage(fs::remove_file(path))?;
                }
            }
            return Err(IngestError::StorageDetails("任务已取消。".into()));
        }
        // Persist intent before changing the current file; restart completes a confirmed edit.
        self.audit(
            if preview.crop.is_some() {
                "crop_commit_pending"
            } else {
                "inpainting_commit_pending"
            },
            &storage(serde_json::to_string(&journal))?,
        )?;
        let intent = directory.join(".repair-journal.json");
        let mut file = storage(tempfile::NamedTempFile::new_in(&directory))?;
        storage(file.write_all(storage(serde_json::to_string(&journal))?.as_bytes()))?;
        storage(file.as_file().sync_all())?;
        storage(file.persist(&intent).map_err(|error| error.error))?;
        self.finish_repair(&journal)?;
        Ok(journal.media)
    }

    fn finish_repair(&mut self, journal: &Journal) -> Result<(), IngestError> {
        let media = &journal.media;
        let directory = media.path.parent().ok_or(IngestError::Storage)?;
        let record = media.edit.as_ref().ok_or(IngestError::InvalidMedia)?;
        let input = journal.parent.as_ref().map_or(media, |parent| parent);
        if input.id != media.id
            || input.path.parent() != Some(directory)
            || (input.path != media.path && input.path.with_extension("png") != media.path)
        {
            return Err(IngestError::InvalidMedia);
        }
        if journal.image != directory.join(".repair-output")
            || journal.thumbnail != directory.join(".repair-thumbnail")
            || if record.operation == "crop" {
                !journal.mask.as_os_str().is_empty()
                    || !record.mask.as_os_str().is_empty()
                    || record.crop.is_none()
                    || record.dimensions.is_none()
            } else {
                record.operation != "inpainting"
                    || journal.mask != directory.join(".repair-mask")
                    || record.mask != directory.join("inpainting-mask.png")
            }
        {
            return Err(IngestError::InvalidMedia);
        }
        if journal.image.is_file() {
            if hash(&journal.image)? != media.sha256
                || (hash(&input.path)? != record.parent_sha256
                    && (input.path != media.path || hash(&input.path)? != media.sha256))
            {
                return Err(IngestError::Integrity);
            }
            validate_png_destination(media, input)?;
            let output = storage(image::load_from_memory(&storage(fs::read(&journal.image))?))?;
            if output.dimensions() != media.dimensions() {
                return Err(IngestError::Integrity);
            }
        } else if hash(&media.path)? != media.sha256 {
            return Err(IngestError::Integrity);
        }
        let mut promotions = vec![
            (&journal.image, &media.path),
            (&journal.thumbnail, &media.thumbnail),
        ];
        let source_target = directory.join(".edit-source");
        promotions.push((&journal.source, &source_target));
        if record.operation == "inpainting" {
            promotions.push((&journal.mask, &record.mask));
        }
        for (source, target) in promotions {
            if source.parent() != Some(directory) || target.parent() != Some(directory) {
                return Err(IngestError::InvalidMedia);
            }
            if source.is_file() {
                storage(
                    tempfile::NamedTempFile::new_in(directory).and_then(|mut file| {
                        file.write_all(&fs::read(source)?)?;
                        file.as_file().sync_all()?;
                        file.persist(target).map_err(|e| e.error)?;
                        Ok(())
                    }),
                )?;
                if source != target {
                    storage(fs::remove_file(source))?;
                }
            }
        }
        if hash(&media.path)? != media.sha256 {
            return Err(IngestError::Integrity);
        }
        if media.current_extension() == "png" {
            staged(
                &directory.join(".png-output.json"),
                &storage(serde_json::to_vec(
                    &serde_json::json!({"media_id":media.id,"path":media.path,"sha256":media.sha256}),
                ))?,
            )?;
        }
        let tx = storage(
            self.connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
        )?;
        storage(tx.execute(
            "UPDATE media SET metadata=?2 WHERE id=?1",
            params![media.id, storage(serde_json::to_string(media))?],
        ))?;
        for table in ["visual_features", "visual_buckets", "media_quality"] {
            storage(tx.execute(
                &format!("DELETE FROM {table} WHERE media_id=?1"),
                [media.id],
            ))?;
        }
        storage(tx.execute(
            "DELETE FROM duplicate_candidates WHERE first_id=?1 OR second_id=?1",
            [media.id],
        ))?;
        storage(tx.execute(
            "UPDATE analysis_jobs SET state='pending',reason='image edited' WHERE media_id=?1",
            [media.id],
        ))?;
        storage(tx.execute(
            "INSERT INTO audit_log(action,metadata) VALUES(?1,?2)",
            params![
                if record.operation == "crop" {
                    "crop_commit"
                } else {
                    "inpainting_commit"
                },
                storage(serde_json::to_string(&media.edit))?
            ],
        ))?;
        storage(tx.commit())?;
        if let Some(parent) = &journal.parent {
            staged(
                &directory.join(".edit-parent.json"),
                &storage(serde_json::to_vec(parent))?,
            )?;
        }
        let old_mask = directory.join("inpainting-mask.png");
        if record.operation == "crop" && old_mask.is_file() {
            storage(fs::remove_file(old_mask))?;
        }
        let intent = directory.join(".repair-journal.json");
        if intent.is_file() {
            storage(fs::remove_file(intent))?;
        }
        Ok(())
    }

    pub fn restore_last_edit(&mut self, id: i64) -> Result<StoredMedia, IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        let mut media = self.media_by_id(id)?.ok_or(IngestError::InvalidMedia)?;
        let edit = media.edit.clone().ok_or(IngestError::InvalidMedia)?;
        let directory = media.path.parent().ok_or(IngestError::Storage)?.to_owned();
        let source = directory.join(".edit-source");
        if !source.is_file() || hash(&source)? != edit.parent_sha256 {
            return Err(IngestError::Integrity);
        }
        let bytes = storage(fs::read(&source))?;
        let restored = storage(image::load_from_memory(&bytes))?;
        let parent_path = directory.join(".edit-parent.json");
        let parent = if parent_path.is_file() {
            let parent: StoredMedia =
                storage(serde_json::from_slice(&storage(fs::read(&parent_path))?))?;
            if parent.id != media.id
                || parent.sha256 != edit.parent_sha256
                || parent.dimensions() != restored.dimensions()
            {
                return Err(IngestError::Integrity);
            }
            Some(parent)
        } else {
            if restored.dimensions() != (media.post.image_width, media.post.image_height) {
                return Err(IngestError::Integrity);
            }
            None
        };
        let thumbnail = restored.thumbnail(320, 320);
        let pending = directory.join(".restore-output");
        let pending_thumbnail = directory.join(".restore-thumbnail");
        staged(&pending, &bytes)?;
        let mut thumb = std::io::Cursor::new(Vec::new());
        storage(thumbnail.write_to(&mut thumb, ImageFormat::Png))?;
        staged(&pending_thumbnail, thumb.get_ref())?;
        let pending_bytes = storage(fs::read(&pending))?;
        let pending_thumbnail_bytes = storage(fs::read(&pending_thumbnail))?;
        let restored_path = parent
            .as_ref()
            .map_or_else(|| media.path.clone(), |parent| parent.path.clone());
        if restored_path.parent() != Some(directory.as_path()) {
            return Err(IngestError::InvalidMedia);
        }
        if restored_path != media.path
            && restored_path.exists()
            && hash(&restored_path)? != edit.parent_sha256
        {
            return Err(IngestError::Integrity);
        }
        storage(
            tempfile::NamedTempFile::new_in(&directory).and_then(|mut file| {
                file.write_all(&pending_bytes)?;
                file.as_file().sync_all()?;
                file.persist(&restored_path).map_err(|e| e.error)?;
                Ok(())
            }),
        )?;
        storage(
            tempfile::NamedTempFile::new_in(&directory).and_then(|mut file| {
                file.write_all(&pending_thumbnail_bytes)?;
                file.as_file().sync_all()?;
                file.persist(&media.thumbnail).map_err(|e| e.error)?;
                Ok(())
            }),
        )?;
        storage(fs::remove_file(&pending))?;
        storage(fs::remove_file(&pending_thumbnail))?;
        storage(fs::remove_file(&source))?;
        media.sha256 = edit.parent_sha256;
        media.path = restored_path;
        media.edit = parent.and_then(|parent| parent.edit);
        if parent_path.is_file() {
            storage(fs::remove_file(parent_path))?;
        }
        let tx = storage(
            self.connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate),
        )?;
        storage(tx.execute(
            "UPDATE media SET metadata=?2 WHERE id=?1",
            params![media.id, storage(serde_json::to_string(&media))?],
        ))?;
        for table in ["visual_features", "visual_buckets", "media_quality"] {
            storage(tx.execute(
                &format!("DELETE FROM {table} WHERE media_id=?1"),
                [media.id],
            ))?;
        }
        storage(tx.execute(
            "UPDATE analysis_jobs SET state='pending',reason='edit restored' WHERE media_id=?1",
            [media.id],
        ))?;
        storage(tx.execute(
            "INSERT INTO audit_log(action,metadata) VALUES('edit_restore',?1)",
            [format!("media_id={}", media.id)],
        ))?;
        storage(tx.commit())?;
        Ok(media)
    }

    pub fn recover_repairs(&mut self) -> Result<(), IngestError> {
        let lock = self.media_lock.clone();
        let _guard = lock.lock().map_err(|_| IngestError::Storage)?;
        let mut query = storage(self.connection.prepare("SELECT metadata FROM media"))?;
        let rows: Vec<String> =
            storage(query.query_map([], |r| r.get(0)).and_then(|r| r.collect()))?;
        drop(query);
        for row in rows {
            let media: StoredMedia = storage(serde_json::from_str(&row))?;
            let intent = media
                .path
                .parent()
                .ok_or(IngestError::Storage)?
                .join(".repair-journal.json");
            if intent.is_file() {
                let journal: Journal =
                    storage(serde_json::from_slice(&storage(fs::read(&intent))?))?;
                if journal.media.id != media.id
                    || (journal.media.path != media.path
                        && journal.parent.as_ref().map(|parent| &parent.path) != Some(&media.path))
                {
                    return Err(IngestError::Integrity);
                }
                self.finish_repair(&journal)?;
            }
        }
        Ok(())
    }
}
