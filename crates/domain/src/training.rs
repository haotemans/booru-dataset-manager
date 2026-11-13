//! Training geometry matching sd-scripts BucketManager(no_upscale=True, reso_steps=32).
use serde::{Deserialize, Serialize};

pub const RESOLUTIONS: [u32; 4] = [768, 1024, 1536, 2048];
pub const STEP: u32 = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrainingCrop {
    pub resolution: u32,
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
    use super::*;

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
