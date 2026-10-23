use bdm_domain::editing::MaskRegion;
use bdm_ports::IngestError;
use image::{GrayImage, Luma};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn rasterize(
    width: u32,
    height: u32,
    regions: &[MaskRegion],
    cancel: &AtomicBool,
) -> Result<GrayImage, IngestError> {
    let mut mask = GrayImage::new(width, height);
    for region in regions {
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        match region {
            MaskRegion::Rectangle { start, end } => {
                paint(&mut mask, &[*start, *end], 0.0, region, cancel)?;
            }
            MaskRegion::Polygon { points } => {
                paint(&mut mask, points, 0.0, region, cancel)?;
            }
            MaskRegion::Brush { points, radius } => {
                if !radius.is_finite() || *radius < 0.0 {
                    return Err(IngestError::InvalidMedia);
                }
                if points.len() == 1 {
                    paint(&mut mask, points, *radius, region, cancel)?;
                }
                // A stroke is the union of its capsules. Only visit pixels near each
                // segment, rather than testing the entire stroke at every image pixel.
                for segment in points.windows(2) {
                    let capsule = MaskRegion::Brush {
                        points: segment.to_vec(),
                        radius: *radius,
                    };
                    paint(&mut mask, segment, *radius, &capsule, cancel)?;
                }
            }
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(IngestError::Cancelled);
    }
    Ok(mask)
}

fn paint(
    mask: &mut GrayImage,
    points: &[[f32; 2]],
    radius: f32,
    region: &MaskRegion,
    cancel: &AtomicBool,
) -> Result<(), IngestError> {
    if points.is_empty() {
        return Ok(());
    }
    if points.iter().flatten().any(|v| !v.is_finite()) {
        return Err(IngestError::InvalidMedia);
    }
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for point in points {
        for axis in 0..2 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    // Conservative bounds include fractional pixel centers and endpoint rounding.
    let left = (min[0] - radius - 1.0)
        .floor()
        .clamp(0.0, mask.width() as f32) as u32;
    let top = (min[1] - radius - 1.0)
        .floor()
        .clamp(0.0, mask.height() as f32) as u32;
    let right = (max[0] + radius + 1.0)
        .ceil()
        .clamp(0.0, mask.width() as f32) as u32;
    let bottom = (max[1] + radius + 1.0)
        .ceil()
        .clamp(0.0, mask.height() as f32) as u32;
    for y in top..bottom {
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        for x in left..right {
            if mask.get_pixel(x, y)[0] == 0 && region.contains(x as f32 + 0.5, y as f32 + 0.5) {
                mask.put_pixel(x, y, Luma([255]));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_masks_match_full_image_geometry() {
        let mut regions = vec![
            MaskRegion::Rectangle {
                start: [33.5, 49.5],
                end: [-3.5, 2.5],
            },
            MaskRegion::Polygon {
                points: vec![[5.5, -2.0], [66.0, 50.5], [30.5, 20.5], [-6.0, 70.0]],
            },
            MaskRegion::Polygon {
                points: vec![[1.0, 1.0], [2.0, 2.0]],
            },
            MaskRegion::Brush {
                points: vec![],
                radius: 2.0,
            },
            MaskRegion::Brush {
                points: vec![[64.5, 0.5]],
                radius: 3.0,
            },
            MaskRegion::Brush {
                points: vec![[10.5, 10.5], [10.5, 10.5], [64.5, 70.5]],
                radius: 0.0,
            },
        ];
        let mut seed = 23_u32;
        for radius in [0.0, 0.5, 2.0, 8.25] {
            let points = (0..40)
                .map(|_| {
                    std::array::from_fn(|_| {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        (seed % 10000) as f32 / 100.0 - 20.0
                    })
                })
                .collect();
            regions.push(MaskRegion::Brush { points, radius });
        }
        // Check each shape independently as well as overlapping unions.
        for shapes in regions
            .iter()
            .map(std::slice::from_ref)
            .chain([regions.as_slice()])
        {
            let actual = rasterize(67, 71, shapes, &AtomicBool::new(false)).unwrap();
            let expected = GrayImage::from_fn(67, 71, |x, y| {
                Luma([u8::from(
                    shapes
                        .iter()
                        .any(|r| r.contains(x as f32 + 0.5, y as f32 + 0.5)),
                ) * 255])
            });
            assert_eq!(actual, expected, "{shapes:?}");
        }
    }

    #[test]
    fn cancellation_and_invalid_coordinates_do_not_produce_a_mask() {
        assert!(matches!(
            rasterize(80, 80, &[], &AtomicBool::new(true)),
            Err(IngestError::Cancelled)
        ));
        for region in [
            MaskRegion::Brush {
                points: vec![[f32::NAN, 1.0]],
                radius: 2.0,
            },
            MaskRegion::Brush {
                points: vec![[1.0, 1.0]],
                radius: f32::INFINITY,
            },
            MaskRegion::Rectangle {
                start: [f32::INFINITY, 1.0],
                end: [2.0, 2.0],
            },
        ] {
            assert!(matches!(
                rasterize(80, 80, &[region], &AtomicBool::new(false)),
                Err(IngestError::InvalidMedia)
            ));
        }
    }
}
