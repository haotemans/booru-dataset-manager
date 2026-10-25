use bdm_ports::IngestError;
use image::{GrayImage, RgbaImage};
use std::sync::atomic::{AtomicBool, Ordering};

const FEATHER: u8 = 16;
const REFERENCE_BAND: u8 = 4;

/// Estimate a smooth spatial color field from the clean selected border only.
/// Never use the watermark-bearing interior as a color reference.
pub(super) fn compose(
    original: &mut RgbaImage,
    repaired: &RgbaImage,
    mask: &GrayImage,
    cancel: &AtomicBool,
) -> Result<(), IngestError> {
    if original.dimensions() != repaired.dimensions() || original.dimensions() != mask.dimensions()
    {
        return Err(IngestError::InvalidMedia);
    }
    let width = mask.width() as usize;
    let height = mask.height() as usize;
    let selected = mask.as_raw();
    let mut depth = vec![FEATHER; selected.len()];
    // Capped Manhattan distance to an unselected pixel. The physical image edge
    // is not a blend boundary: there is no unchanged background beyond it.
    for y in 0..height {
        check_cancel(cancel)?;
        for x in 0..width {
            let i = y * width + x;
            if selected[i] == 0 {
                depth[i] = 0;
            } else {
                if x > 0 {
                    depth[i] = depth[i].min(depth[i - 1] + 1);
                }
                if y > 0 {
                    depth[i] = depth[i].min(depth[i - width] + 1);
                }
            }
        }
    }
    for y in (0..height).rev() {
        check_cancel(cancel)?;
        for x in (0..width).rev() {
            let i = y * width + x;
            if x + 1 < width {
                depth[i] = depth[i].min(depth[i + 1] + 1);
            }
            if y + 1 < height {
                depth[i] = depth[i].min(depth[i + width] + 1);
            }
        }
    }

    let mut visited = vec![false; selected.len()];
    let mut component = Vec::new();
    let source = original.as_mut();
    let repair = repaired.as_raw();
    for start in 0..selected.len() {
        if start % 4096 == 0 {
            check_cancel(cancel)?;
        }
        if selected[start] == 0 || visited[start] {
            continue;
        }
        component.clear();
        component.push(start);
        visited[start] = true;
        let mut cursor = 0;
        let mut max_depth = 1;
        let mut residuals = [[0u32; 511]; 3];
        let mut samples = 0;
        while cursor < component.len() {
            if cursor % 4096 == 0 {
                check_cancel(cancel)?;
            }
            let i = component[cursor];
            cursor += 1;
            max_depth = max_depth.max(depth[i]);
            // Ignore transparent RGB values, which need not represent visible
            // background. Median statistics resist small border contaminants.
            if depth[i] <= REFERENCE_BAND && source[i * 4 + 3] == 255 {
                for channel in 0..3 {
                    let difference =
                        i16::from(source[i * 4 + channel]) - i16::from(repair[i * 4 + channel]);
                    residuals[channel][(difference + 255) as usize] += 1;
                }
                samples += 1;
            }
            let x = i % width;
            let y = i / width;
            for neighbor in [
                (x > 0).then(|| i - 1),
                (x + 1 < width).then_some(i + 1),
                (y > 0).then(|| i - width),
                (y + 1 < height).then_some(i + width),
            ]
            .into_iter()
            .flatten()
            {
                if selected[neighbor] > 0 && !visited[neighbor] {
                    visited[neighbor] = true;
                    component.push(neighbor);
                }
            }
        }
        // Thin strokes cannot provide a reliable clean margin. They still get
        // full-strength repair at their center, including one-pixel masks.
        let field = if max_depth >= REFERENCE_BAND * 2 && samples >= 64 {
            let offset = residuals.map(|histogram| median(&histogram, samples));
            Some(ColorField::fit(
                &component, width, &depth, source, repair, offset, cancel,
            )?)
        } else {
            None
        };
        for (n, &i) in component.iter().enumerate() {
            if n % 4096 == 0 {
                check_cancel(cancel)?;
            }
            let t = f32::from(depth[i]) / f32::from(max_depth);
            let weight = t * t * (3.0 - 2.0 * t);
            let offset = field
                .as_ref()
                .map_or([0.0; 3], |field| field.at(i % width, i / width));
            for channel in 0..3 {
                let before = f32::from(source[i * 4 + channel]);
                let after =
                    (f32::from(repair[i * 4 + channel]) + offset[channel]).clamp(0.0, 255.0);
                source[i * 4 + channel] = (before + (after - before) * weight).round() as u8;
            }
        }
    }
    check_cancel(cancel)
}

struct ColorField {
    left: usize,
    top: usize,
    step: usize,
    width: usize,
    height: usize,
    values: Vec<[f32; 3]>,
}

impl ColorField {
    fn fit(
        component: &[usize],
        image_width: usize,
        depth: &[u8],
        source: &[u8],
        repair: &[u8],
        baseline: [f32; 3],
        cancel: &AtomicBool,
    ) -> Result<Self, IngestError> {
        let left = component.iter().map(|i| i % image_width).min().unwrap();
        let right = component.iter().map(|i| i % image_width).max().unwrap();
        let top = component.iter().map(|i| i / image_width).min().unwrap();
        let bottom = component.iter().map(|i| i / image_width).max().unwrap();
        // Bounded coarse solve keeps composition cheap on full-resolution images.
        // Median bins reject isolated remnants; harmonic extension changes tone
        // smoothly without copying original texture or central watermark pixels.
        let step = ((right - left).max(bottom - top) / 64).max(4);
        let width = (right - left).div_ceil(step) + 1;
        let height = (bottom - top).div_ceil(step) + 1;
        let mut samples = vec![Vec::<[i16; 3]>::new(); width * height];
        for (n, &i) in component.iter().enumerate() {
            if n % 4096 == 0 {
                check_cancel(cancel)?;
            }
            if depth[i] > REFERENCE_BAND || source[i * 4 + 3] != 255 {
                continue;
            }
            let residual = std::array::from_fn(|c| {
                i16::from(source[i * 4 + c]) - i16::from(repair[i * 4 + c])
            });
            // Reject gross contaminants jointly, rather than accepting their
            // unaffected channels as independent evidence.
            if (0..3).any(|c| (f32::from(residual[c]) - baseline[c]).abs() > 48.0) {
                continue;
            }
            let x = (i % image_width - left + step / 2) / step;
            let y = (i / image_width - top + step / 2) / step;
            samples[y * width + x].push(residual);
        }
        let mut values = vec![baseline.map(|v| v.clamp(-96.0, 96.0)); width * height];
        let mut fixed = vec![false; values.len()];
        for (i, bin) in samples.iter().enumerate() {
            if bin.is_empty() {
                continue;
            }
            fixed[i] = true;
            for c in 0..3 {
                let mut channel: Vec<i16> = bin.iter().map(|v| v[c]).collect();
                channel.sort_unstable();
                values[i][c] = ((f32::from(channel[(channel.len() - 1) / 2])
                    + f32::from(channel[channel.len() / 2]))
                    * 0.5)
                    .clamp(-96.0, 96.0);
            }
        }
        for _ in 0..2048 {
            check_cancel(cancel)?;
            let mut largest = 0.0f32;
            for y in 0..height {
                for x in 0..width {
                    let i = y * width + x;
                    if fixed[i] {
                        continue;
                    }
                    let mut sum = [0.0; 3];
                    let mut count = 0.0;
                    for neighbor in [
                        (x > 0).then(|| i - 1),
                        (x + 1 < width).then_some(i + 1),
                        (y > 0).then(|| i - width),
                        (y + 1 < height).then_some(i + width),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        for c in 0..3 {
                            sum[c] += values[neighbor][c];
                        }
                        count += 1.0;
                    }
                    for c in 0..3 {
                        let next = sum[c] / count;
                        largest = largest.max((next - values[i][c]).abs());
                        values[i][c] = next;
                    }
                }
            }
            if largest < 0.002 {
                break;
            }
        }
        Ok(Self {
            left,
            top,
            step,
            width,
            height,
            values,
        })
    }

    fn at(&self, x: usize, y: usize) -> [f32; 3] {
        let gx = (x - self.left) as f32 / self.step as f32;
        let gy = (y - self.top) as f32 / self.step as f32;
        let x0 = (gx as usize).min(self.width - 1);
        let y0 = (gy as usize).min(self.height - 1);
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        std::array::from_fn(|c| {
            let a = self.values[y0 * self.width + x0][c] * (1.0 - gx.fract())
                + self.values[y0 * self.width + x1][c] * gx.fract();
            let b = self.values[y1 * self.width + x0][c] * (1.0 - gx.fract())
                + self.values[y1 * self.width + x1][c] * gx.fract();
            a * (1.0 - gy.fract()) + b * gy.fract()
        })
    }
}

fn median(histogram: &[u32; 511], count: u32) -> f32 {
    let mut cumulative = 0;
    let mut lower = None;
    for (value, &frequency) in histogram.iter().enumerate() {
        cumulative += frequency;
        if cumulative > (count - 1) / 2 && lower.is_none() {
            lower = Some(value);
        }
        if cumulative > count / 2 {
            return (lower.unwrap() + value) as f32 * 0.5 - 255.0;
        }
    }
    0.0
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), IngestError> {
    if cancel.load(Ordering::Relaxed) {
        Err(IngestError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Luma, Rgba};

    fn apply(original: &mut RgbaImage, repaired: &RgbaImage, mask: &GrayImage) {
        compose(original, repaired, mask, &AtomicBool::new(false)).unwrap();
    }

    #[test]
    fn matches_background_without_copying_central_watermark_or_losing_texture() {
        let clean = RgbaImage::from_fn(120, 100, |x, y| {
            let texture = ((x * 7 + y * 3) % 9) as u8;
            Rgba([140 + x as u8 / 4 + texture, 170 + y as u8 / 8, 190, 255])
        });
        let mask = GrayImage::from_fn(120, 100, |x, y| {
            Luma([u8::from((10..110).contains(&x) && (10..90).contains(&y)) * 255])
        });
        let repaired = RgbaImage::from_fn(120, 100, |x, y| {
            let p = clean.get_pixel(x, y);
            Rgba([p[0] - 12, p[1] + 8, p[2] - 5, 123])
        });
        let mut original = clean.clone();
        for y in 35..65 {
            for x in 35..85 {
                original.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        // A few residual watermark pixels on the border must not bias the fit.
        for x in 10..14 {
            original.put_pixel(x, 20, Rgba([255, 255, 255, 255]));
        }
        apply(&mut original, &repaired, &mask);
        for y in 30..70 {
            for x in 30..90 {
                assert_eq!(original.get_pixel(x, y), clean.get_pixel(x, y));
            }
        }
        for x in 20..100 {
            assert_eq!(original.get_pixel(x, 10), clean.get_pixel(x, 10));
        }
    }

    #[test]
    fn spatial_color_drift_is_corrected_throughout_the_interior() {
        let clean = RgbaImage::from_fn(160, 120, |x, y| {
            Rgba([145 + ((x * 3 + y * 7) % 7) as u8, 155, 165, 255])
        });
        let mask = GrayImage::from_fn(160, 120, |x, y| {
            Luma([u8::from((8..152).contains(&x) && (8..112).contains(&y)) * 255])
        });
        let repaired = RgbaImage::from_fn(160, 120, |x, y| {
            let p = clean.get_pixel(x, y);
            Rgba([p[0] - (38 + x / 5) as u8, p[1] + (y / 4) as u8, p[2], 0])
        });
        let mut original = clean.clone();
        for y in 40..80 {
            for x in 40..120 {
                original.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        apply(&mut original, &repaired, &mask);
        let mut error = 0u64;
        let mut count = 0u64;
        for (x, y, pixel) in original.enumerate_pixels() {
            if mask.get_pixel(x, y)[0] == 0 {
                assert_eq!(pixel, clean.get_pixel(x, y));
            }
            if (30..130).contains(&x) && (30..90).contains(&y) {
                for c in 0..3 {
                    let difference = pixel[c].abs_diff(clean.get_pixel(x, y)[c]);
                    assert!(difference <= 3, "interior drift at {x},{y}: {difference}");
                    error += u64::from(difference);
                    count += 1;
                }
            }
        }
        assert!((error as f64 / count as f64) < 1.0);
    }

    #[test]
    fn independent_regions_match_their_own_background_and_preserve_outside_and_alpha() {
        let mut original = RgbaImage::from_fn(180, 100, |x, y| {
            Rgba([
                120 + (x / 90) as u8 * 60,
                145,
                160,
                if y < 15 { 77 } else { 255 },
            ])
        });
        let before = original.clone();
        let repaired = RgbaImage::from_fn(180, 100, |x, _| {
            Rgba([if x < 90 { 105 } else { 200 }, 145, 160, 0])
        });
        let mask = GrayImage::from_fn(180, 100, |x, y| {
            Luma([u8::from(
                ((10..80).contains(&x) || (100..170).contains(&x)) && (5..90).contains(&y),
            ) * 255])
        });
        apply(&mut original, &repaired, &mask);
        assert_eq!(original, before);
    }

    #[test]
    fn edge_transition_is_smooth_but_small_masks_keep_full_strength() {
        let mut original = RgbaImage::from_pixel(100, 100, Rgba([100, 100, 100, 255]));
        let repaired = RgbaImage::from_pixel(100, 100, Rgba([200, 200, 200, 0]));
        let mask = GrayImage::from_fn(100, 100, |x, y| {
            Luma([u8::from((10..90).contains(&x) && (10..90).contains(&y)) * 255])
        });
        apply(&mut original, &repaired, &mask);
        assert!(original.get_pixel(10, 50)[0] <= 101);
        for x in 10..30 {
            let step = i16::from(original.get_pixel(x, 50)[0])
                - i16::from(original.get_pixel(x - 1, 50)[0]);
            assert!((0..=7).contains(&step), "hard seam at {x}: {step}");
        }
        for side in 1..=7 {
            let mut original = RgbaImage::from_pixel(20, 20, Rgba([100, 100, 100, 70]));
            let repaired = RgbaImage::from_pixel(20, 20, Rgba([200, 200, 200, 0]));
            let mask = GrayImage::from_fn(20, 20, |x, y| {
                Luma([u8::from((5..5 + side).contains(&x) && (5..5 + side).contains(&y)) * 255])
            });
            apply(&mut original, &repaired, &mask);
            assert_eq!(
                original.get_pixel(5 + side / 2, 5 + side / 2),
                &Rgba([200, 200, 200, 70])
            );
        }
    }

    #[test]
    fn holes_image_edges_full_masks_and_cancellation_are_handled() {
        let original = RgbaImage::from_pixel(64, 64, Rgba([100, 100, 100, 99]));
        let repaired = RgbaImage::from_pixel(64, 64, Rgba([180, 180, 180, 0]));
        let full = GrayImage::from_pixel(64, 64, Luma([255]));
        let mut result = original.clone();
        apply(&mut result, &repaired, &full);
        assert!(result.pixels().all(|p| p == &Rgba([180, 180, 180, 99])));
        let mut mask = full;
        for y in 24..40 {
            for x in 24..40 {
                mask.put_pixel(x, y, Luma([0]));
            }
        }
        result = original.clone();
        apply(&mut result, &repaired, &mask);
        assert_eq!(result.get_pixel(0, 0), &Rgba([180, 180, 180, 99]));
        assert_eq!(result.get_pixel(30, 30), original.get_pixel(30, 30));
        assert!(result.get_pixel(23, 30)[0] <= 101);
        assert!(matches!(
            compose(&mut result, &repaired, &mask, &AtomicBool::new(true)),
            Err(IngestError::Cancelled)
        ));
        let mut unchanged = original.clone();
        apply(&mut unchanged, &repaired, &GrayImage::new(64, 64));
        assert_eq!(unchanged, original);
    }

    #[test]
    #[ignore = "Requires BDM_INPAINT_FIXTURE with original.png, repair.png and mask.png; writes blended.png"]
    fn local_fixture_composition() {
        let directory = std::path::PathBuf::from(
            std::env::var_os("BDM_INPAINT_FIXTURE").expect("fixture directory"),
        );
        let mut original = image::open(directory.join("original.png"))
            .unwrap()
            .to_rgba8();
        let before = original.clone();
        let repaired = image::open(directory.join("repair.png"))
            .unwrap()
            .to_rgba8();
        let mask = image::open(directory.join("mask.png")).unwrap().to_luma8();
        let started = std::time::Instant::now();
        apply(&mut original, &repaired, &mask);
        println!("Composition: {:?}", started.elapsed());
        for (x, y, p) in original.enumerate_pixels() {
            assert_eq!(p[3], before.get_pixel(x, y)[3]);
            if mask.get_pixel(x, y)[0] == 0 {
                assert_eq!(p, before.get_pixel(x, y));
            }
        }
        original.save(directory.join("blended.png")).unwrap();
    }
}
