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
    collections::VecDeque,
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
        }
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
    fn edited_jpeg_webp_and_png_retain_source_format_and_pixel_dimensions() {
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
            let preview = synthetic(&media);
            let edited = f.repo.commit_repair(media.id, &preview).unwrap();
            assert_eq!(
                image::guess_format(&fs::read(&edited.path).unwrap()).unwrap(),
                format
            );
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
                format
            );
            assert_eq!(image::open(&edited.path).unwrap().dimensions(), (600, 400));
        }
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
    })
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
    // Preserve unselected pixels exactly and feather the repair at the mask edge.
    let mut composed = decoded.to_rgba8();
    let repair = result.to_rgba8();
    let weights = mask_edge_weights(&mask_image);
    for (x, y, pixel) in composed.enumerate_pixels_mut() {
        if mask_image.get_pixel(x, y)[0] > 0 {
            let alpha = pixel[3];
            let feather = weights[(y * width + x) as usize];
            let repaired = repair.get_pixel(x, y);
            for channel in 0..3 {
                pixel[channel] = ((f32::from(pixel[channel]) * (1.0 - feather)
                    + f32::from(repaired[channel]) * feather)
                    .round()
                    .clamp(0.0, 255.0)) as u8;
            }
            pixel[3] = alpha;
        }
    }
    storage(composed.save(&output))?;
    Ok(RepairPreview {
        directory,
        image: output,
        mask,
        parent_sha256,
        model: if model == "anime-lama" {
            "anime-lama@b592884b2b6589e57df6916db6bcac7a6307fc58:sha256-95d4bf7eb2f13729351ac0a0693c9b9b469b264fc1542ca3fa551b59b464a160:crop128-max1024-v1".into()
        } else {
            "lama:md5-e3aa4aaa15225a33ec84f9f4bc47e500:crop128-max1024-v1".into()
        },
        regions,
        crop: None,
    })
}

fn mask_edge_weights(mask: &image::GrayImage) -> Vec<f32> {
    const FEATHER: i32 = 8;
    let width = mask.width() as i32;
    let height = mask.height() as i32;
    let mut distances = vec![FEATHER + 1; (width * height) as usize];
    let mut queue = VecDeque::new();
    for y in 0..height {
        for x in 0..width {
            if mask.get_pixel(x as u32, y as u32)[0] == 0 {
                continue;
            }
            let edge = [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
                .iter()
                .any(|&(nx, ny)| {
                    nx < 0
                        || ny < 0
                        || nx >= width
                        || ny >= height
                        || mask.get_pixel(nx as u32, ny as u32)[0] == 0
                });
            if edge {
                distances[(y * width + x) as usize] = 1;
                queue.push_back((x, y));
            }
        }
    }
    while let Some((x, y)) = queue.pop_front() {
        let distance = distances[(y * width + x) as usize];
        if distance >= FEATHER {
            continue;
        }
        for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
            if nx < 0 || ny < 0 || nx >= width || ny >= height {
                continue;
            }
            let index = (ny * width + nx) as usize;
            if mask.get_pixel(nx as u32, ny as u32)[0] > 0 && distances[index] > distance + 1 {
                distances[index] = distance + 1;
                queue.push_back((nx, ny));
            }
        }
    }
    distances
        .into_iter()
        .map(|distance| {
            if distance > FEATHER {
                1.0
            } else {
                // Keep most of the repair at the edge while removing a hard color seam.
                (0.45 + 0.55 * distance as f32 / FEATHER as f32).clamp(0.0, 1.0)
            }
        })
        .collect()
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Journal {
    media: StoredMedia,
    image: PathBuf,
    thumbnail: PathBuf,
    mask: PathBuf,
    source: PathBuf,
}

fn staged(path: &Path, bytes: &[u8]) -> Result<(), IngestError> {
    let mut file = storage(fs::File::create(path))?;
    storage(file.write_all(bytes))?;
    storage(file.sync_all())
}

impl SqliteCatalog {
    pub fn redownload_original(
        &mut self,
        id: i64,
        provider: &dyn MediaProvider,
        cancel: &AtomicBool,
    ) -> Result<StoredMedia, IngestError> {
        let media = self.media_by_id(id)?.ok_or(IngestError::InvalidMedia)?;
        if media.edit.is_some() {
            return Err(IngestError::StorageDetails(
                "当前图片有未完成编辑，请先恢复编辑前版本。".into(),
            ));
        }
        let directory = media.path.parent().ok_or(IngestError::Storage)?;
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
        if bytes.len() as u64 != media.post.file_size || bytes.len() > 64 * 1024 * 1024 {
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
        if current.sha256 != media.sha256 || current.edit.is_some() {
            return Err(IngestError::StorageDetails(
                "图片在下载期间发生变化，请重新执行下载。".into(),
            ));
        }
        if media.path.is_file() {
            storage(fs::copy(&media.path, &backup))?;
        }
        let replacement = storage(tempfile::NamedTempFile::new_in(directory))?;
        storage(fs::copy(&staged_image, replacement.path()))?;
        storage(replacement.persist(&media.path).map_err(|e| e.error))?;
        let replacement_thumbnail = storage(tempfile::NamedTempFile::new_in(directory))?;
        storage(fs::copy(&staged_thumbnail, replacement_thumbnail.path()))?;
        storage(
            replacement_thumbnail
                .persist(&media.thumbnail)
                .map_err(|e| e.error),
        )?;

        let mut updated = media;
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
        if media.sha256 != preview.parent_sha256 || hash(&media.path)? != preview.parent_sha256 {
            return Err(IngestError::Integrity);
        }
        let directory = media.path.parent().ok_or(IngestError::Storage)?.to_owned();
        let image = storage(image::open(&preview.image))?;
        if let Some(crop) = preview.crop {
            if !crop.valid(media.dimensions()) || image.dimensions() != (crop.width, crop.height) {
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
        let format = match media.post.file_ext.as_str() {
            "jpg" | "jpeg" => ImageFormat::Jpeg,
            "png" => ImageFormat::Png,
            "webp" => ImageFormat::WebP,
            _ => return Err(IngestError::InvalidMedia),
        };
        let mut bytes = std::io::Cursor::new(Vec::new());
        storage(image.write_to(&mut bytes, format))?;
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
        media.sha256 = format!("{:x}", Sha256::digest(&bytes));
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
        });
        let journal = Journal {
            media,
            image: pending,
            thumbnail,
            mask,
            source,
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
                || hash(&media.path)? != record.parent_sha256 && hash(&media.path)? != media.sha256
            {
                return Err(IngestError::Integrity);
            }
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
                storage(fs::remove_file(source))?;
            }
        }
        if hash(&media.path)? != media.sha256 {
            return Err(IngestError::Integrity);
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
        let directory = media.path.parent().ok_or(IngestError::Storage)?;
        let source = directory.join(".edit-source");
        if !source.is_file() || hash(&source)? != edit.parent_sha256 {
            return Err(IngestError::Integrity);
        }
        let bytes = storage(fs::read(&source))?;
        let restored = storage(image::load_from_memory(&bytes))?;
        let thumbnail = restored.thumbnail(320, 320);
        let pending = directory.join(".restore-output");
        let pending_thumbnail = directory.join(".restore-thumbnail");
        staged(&pending, &bytes)?;
        let mut thumb = std::io::Cursor::new(Vec::new());
        storage(thumbnail.write_to(&mut thumb, ImageFormat::Png))?;
        staged(&pending_thumbnail, thumb.get_ref())?;
        let pending_bytes = storage(fs::read(&pending))?;
        let pending_thumbnail_bytes = storage(fs::read(&pending_thumbnail))?;
        storage(
            tempfile::NamedTempFile::new_in(directory).and_then(|mut file| {
                file.write_all(&pending_bytes)?;
                file.as_file().sync_all()?;
                file.persist(&media.path).map_err(|e| e.error)?;
                Ok(())
            }),
        )?;
        storage(
            tempfile::NamedTempFile::new_in(directory).and_then(|mut file| {
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
        media.edit = None;
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
                if journal.media.id != media.id || journal.media.path != media.path {
                    return Err(IngestError::Integrity);
                }
                self.finish_repair(&journal)?;
            }
        }
        Ok(())
    }
}
