//! Training geometry matching sd-scripts BucketManager(no_upscale=True, reso_steps=32).
use serde::{Deserialize, Serialize};

pub const RESOLUTIONS: [u32; 4] = [768, 1024, 1536, 2048];
pub const STEP: u32 = 32;
pub const RATIOS: [(&str, u32, u32); 17] = [
    ("21:9", 21, 9),
    ("2:1", 2, 1),
    ("16:9", 16, 9),
    ("16:10", 16, 10),
    ("3:2", 3, 2),
    ("7:5", 7, 5),
    ("4:3", 4, 3),
    ("5:4", 5, 4),
    ("1:1", 1, 1),
    ("4:5", 4, 5),
    ("3:4", 3, 4),
    ("5:7", 5, 7),
    ("2:3", 2, 3),
    ("10:16", 10, 16),
    ("9:16", 9, 16),
    ("1:2", 1, 2),
    ("9:21", 9, 21),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketChoice {
    pub ratio: String,
    /// Native-pixel candidate, centered within the supplied region. No resizing.
    pub native_crop: [u32; 4],
    pub resolution: u32,
    pub training: TrainingCrop,
    pub retained_fraction: f64,
    pub training_resize_required: bool,
    pub extra_training_crop: bool,
}

/// Use the actual no-upscale BucketManager geometry, not a tier label alone.
/// The Agent chooses composition; this pure function never touches image files.
pub fn best_bucket(width: u32, height: u32, ratio: &str) -> Result<BucketChoice, &'static str> {
    let target_aspect = RATIOS
        .iter()
        .find(|r| r.0 == ratio)
        .map(|r| f64::from(r.1) / f64::from(r.2));
    let (w, h) = if ratio == "original" {
        (width / STEP * STEP, height / STEP * STEP)
    } else {
        let (_, rw, rh) = RATIOS
            .iter()
            .find(|(name, _, _)| *name == ratio)
            .ok_or("未知比例；请使用现有比例。")?;
        if u64::from(width) * u64::from(*rh) > u64::from(height) * u64::from(*rw) {
            (
                (u64::from(height) * u64::from(*rw) / u64::from(*rh)) as u32 / STEP * STEP,
                height / STEP * STEP,
            )
        } else {
            (
                width / STEP * STEP,
                (u64::from(width) * u64::from(*rh) / u64::from(*rw)) as u32 / STEP * STEP,
            )
        }
    };
    if w.min(h) < 576 || u64::from(w) * u64::from(h) < 550_000 {
        return Err("选区不足短边 576 px／0.55 MP，不会放大。");
    }
    let mut tiers = RESOLUTIONS;
    let area = f64::from(w) * f64::from(h);
    tiers.sort_by(|a, b| {
        (area / f64::from(*a).powi(2))
            .ln()
            .abs()
            .total_cmp(&(area / f64::from(*b).powi(2)).ln().abs())
    });
    for tier in tiers {
        let mut best: Option<BucketChoice> = None;
        for dx in 0..=16_u32.min((w - 576) / STEP) {
            for dy in 0..=16_u32.min((h - 576) / STEP) {
                let (cw, ch) = (w - dx * STEP, h - dy * STEP);
                // Presets are nominal training ratios on a 32-pixel grid. Do
                // not silently drift into a different ratio to find a bucket.
                if target_aspect.is_some_and(|target| {
                    (f64::from(cw) / f64::from(ch) / target - 1.0).abs() > 0.02
                }) {
                    continue;
                }
                let Ok(training) = plan(cw, ch, tier) else {
                    continue;
                };
                if training.resized != training.output {
                    continue;
                }
                let fraction =
                    f64::from(cw) * f64::from(ch) / (f64::from(width) * f64::from(height));
                if best.as_ref().is_none_or(|b| fraction > b.retained_fraction) {
                    best = Some(BucketChoice {
                        ratio: ratio.into(),
                        native_crop: [(width - cw) / 2, (height - ch) / 2, cw, ch],
                        resolution: tier,
                        training_resize_required: training.resized != [cw, ch],
                        training,
                        retained_fraction: fraction,
                        extra_training_crop: false,
                    });
                }
            }
        }
        if let Some(choice) = best {
            return Ok(choice);
        }
    }
    Err("当前选区未找到可复核的分桶方案；请调整现有比例或选区，不会自动缩放原像素图片。")
}

pub fn bucket_choices(width: u32, height: u32) -> Vec<BucketChoice> {
    std::iter::once("original")
        .chain(RATIOS.iter().map(|r| r.0))
        .filter_map(|r| best_bucket(width, height, r).ok())
        .collect()
}

/// Native crop: preserve every retained pixel and trim only for 32 alignment.
/// Resolution is informational; explicit export variants use `plan` instead.
pub fn automatic_plan(width: u32, height: u32) -> Result<TrainingCrop, &'static str> {
    let output = [width / STEP * STEP, height / STEP * STEP];
    if output[0].min(output[1]) < 576 || u64::from(output[0]) * u64::from(output[1]) < 550_000 {
        return Err("选区对齐后不足短边 576 px／0.55 MP；不会放大补足。");
    }
    Ok(TrainingCrop {
        resolution: nearest_tier(output[0], output[1]),
        native: true,
        resized: [width, height],
        output,
        offset: [(width - output[0]) / 2, (height - output[1]) / 2],
    })
}

pub fn verify(width: u32, height: u32, saved: &TrainingCrop) -> Result<TrainingCrop, &'static str> {
    if saved.native {
        automatic_plan(width, height)
    } else {
        plan(width, height, saved.resolution)
    }
}

pub fn nearest_tier(width: u32, height: u32) -> u32 {
    let equivalent = (f64::from(width) * f64::from(height)).sqrt();
    RESOLUTIONS
        .into_iter()
        .min_by(|a, b| {
            (f64::from(*a) - equivalent)
                .abs()
                .total_cmp(&(f64::from(*b) - equivalent).abs())
        })
        .unwrap()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrainingCrop {
    pub resolution: u32,
    #[serde(default)]
    pub native: bool,
    pub resized: [u32; 2],
    pub output: [u32; 2],
    pub offset: [u32; 2],
}

fn geometry(width: u32, height: u32, resolution: u32) -> Option<TrainingCrop> {
    if width < STEP || height < STEP || !RESOLUTIONS.contains(&resolution) {
        return None;
    }
    let area = u64::from(resolution).pow(2);
    let aspect = f64::from(width) / f64::from(height);
    let round = |value: f64| ((value + 0.5) as u32 / STEP) * STEP;
    let resized = if u64::from(width) * u64::from(height) > area {
        let w = (area as f64 * aspect).sqrt();
        let h = area as f64 / w;
        let bw = round(w);
        let bhw = round(f64::from(bw) / aspect);
        let bh = round(h);
        let bwh = round(f64::from(bh) * aspect);
        if [bw, bhw, bh, bwh].contains(&0) {
            return None;
        }
        if (f64::from(bw) / f64::from(bhw) - aspect).abs()
            < (f64::from(bwh) / f64::from(bh) - aspect).abs()
        {
            [bw, (f64::from(bw) / aspect + 0.5) as u32]
        } else {
            [(f64::from(bh) * aspect + 0.5) as u32, bh]
        }
    } else {
        [width, height]
    };
    let output = resized.map(|v| v / STEP * STEP);
    if output.contains(&0) || resized[0] > width || resized[1] > height {
        return None;
    }
    Some(TrainingCrop {
        resolution,
        native: false,
        resized,
        output,
        offset: [(resized[0] - output[0]) / 2, (resized[1] - output[1]) / 2],
    })
}

pub fn plan(width: u32, height: u32, resolution: u32) -> Result<TrainingCrop, &'static str> {
    let result = geometry(width, height, resolution).ok_or("选区尺寸或训练档位无效。")?;
    if width.min(height) < 576 || u64::from(width) * u64::from(height) < 550_000 {
        return Err("选区不足训练收录下限：短边至少 576 px、像素至少 0.55 MP；不会放大补足。 ");
    }
    if result.output[0].min(result.output[1]) < 576
        || u64::from(result.output[0]) * u64::from(result.output[1]) < 550_000
    {
        return Err(
            "此比例在当前档位对齐后不足短边 576 px／0.55 MP，请提高上限或调整选区；不会放大。 ",
        );
    }
    let repeat = geometry(result.output[0], result.output[1], resolution)
        .ok_or("训练尺寸无法通过分桶复核。")?;
    if repeat.output != result.output || repeat.resized != result.output {
        return Err("训练尺寸会被脚本再次调整，请更换档位或选区。 ");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    #[test]
    fn geometry_matches_outputs_from_local_sd_scripts() {
        for line in include_str!("../tests/fixtures/kohya-no-upscale32.csv")
            .lines()
            .filter(|s| !s.starts_with('#') && !s.is_empty())
        {
            let v: Vec<u32> = line.split(',').map(|s| s.parse().unwrap()).collect();
            let p = geometry(v[0], v[1], v[2]).unwrap();
            assert_eq!(p.output, [v[3], v[4]], "{line}");
            assert_eq!(p.resized, [v[5], v[6]], "{line}");
        }
    }
    #[test]
    fn recommended_native_crops_match_kohya_without_another_crop() {
        for (w, h) in [
            (800, 800),
            (2160, 1509),
            (2702, 4096),
            (8507, 3902),
            (9746, 5482),
        ] {
            let original = best_bucket(w, h, "original").unwrap();
            assert!(original.retained_fraction > 0.7);
            for choice in bucket_choices(w, h) {
                let [x, y, cw, ch] = choice.native_crop;
                if choice.ratio != "original" {
                    let (_, rw, rh) = RATIOS.iter().find(|r| r.0 == choice.ratio).unwrap();
                    assert!(
                        (f64::from(cw) / f64::from(ch) / (f64::from(*rw) / f64::from(*rh)) - 1.0)
                            .abs()
                            <= 0.02
                    );
                }
                assert!(x + cw <= w && y + ch <= h);
                assert_eq!([cw % 32, ch % 32], [0, 0]);
                assert!(cw.min(ch) >= 576 && u64::from(cw) * u64::from(ch) >= 550_000);
                let actual = plan(cw, ch, choice.resolution).unwrap();
                assert_eq!(actual, choice.training);
                assert_eq!(actual.resized, actual.output);
                assert!(actual.output[0] <= cw && actual.output[1] <= ch);
                assert_eq!(
                    plan(actual.output[0], actual.output[1], choice.resolution)
                        .unwrap()
                        .output,
                    actual.output
                );
            }
        }
        assert!(best_bucket(512, 512, "original").is_err());
        assert!(best_bucket(4096, 4096, "unknown").is_err());
    }
    use super::*;

    #[test]
    fn automatic_tiers_keep_native_pixels_even_above_2048() {
        for (size, expected, tier) in [
            ([800, 800], [800, 800], 768),
            ([1120, 896], [1120, 896], 1024),
            ([1536, 1536], [1536, 1536], 1536),
            ([2048, 2048], [2048, 2048], 2048),
            ([2160, 1509], [2144, 1504], 2048),
            ([3000, 3000], [2976, 2976], 2048),
            ([9746, 5482], [9728, 5472], 2048),
        ] {
            let result = automatic_plan(size[0], size[1]).unwrap();
            assert_eq!(result.output, expected);
            assert_eq!(result.resized, size);
            assert!(result.native);
            assert_eq!(nearest_tier(expected[0], expected[1]), tier);
            assert_eq!(
                automatic_plan(expected[0], expected[1]).unwrap().resized,
                expected
            );
        }
        assert!(automatic_plan(512, 512).is_err());
    }

    #[test]
    fn kohya_reference_shapes_are_stable_and_never_upscaled() {
        // Actual outputs from the user's library/dataset.py BucketManager.
        for (size, resolution, expected, resized) in [
            ([896, 1184], 1024, [864, 1152], [872, 1152]),
            ([1344, 1792], 1536, [1312, 1760], [1320, 1760]),
            ([672, 896], 768, [640, 864], [648, 864]),
            ([1664, 2496], 2048, [1664, 2496], [1664, 2496]),
            ([1800, 1800], 1024, [1024, 1024], [1024, 1024]),
            ([800, 800], 2048, [800, 800], [800, 800]),
        ] {
            let result = plan(size[0], size[1], resolution).unwrap();
            assert_eq!(result.output, expected);
            assert_eq!(result.resized, resized);
            assert_eq!(
                plan(expected[0], expected[1], resolution).unwrap().resized,
                expected
            );
        }
        assert!(plan(960, 640, 768).is_err()); // native result would fall below 0.55MP
        assert!(plan(512, 512, 1024).is_err());
    }
}
