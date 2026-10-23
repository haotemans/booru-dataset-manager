use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl CropRect {
    pub fn valid(self, dimensions: (u32, u32)) -> bool {
        self.width > 0
            && self.height > 0
            && self
                .x
                .checked_add(self.width)
                .is_some_and(|v| v <= dimensions.0)
            && self
                .y
                .checked_add(self.height)
                .is_some_and(|v| v <= dimensions.1)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MaskRegion {
    Rectangle { start: [f32; 2], end: [f32; 2] },
    Brush { points: Vec<[f32; 2]>, radius: f32 },
    Polygon { points: Vec<[f32; 2]> },
}

impl MaskRegion {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        match self {
            Self::Rectangle { start, end } => {
                x >= start[0].min(end[0])
                    && x < start[0].max(end[0])
                    && y >= start[1].min(end[1])
                    && y < start[1].max(end[1])
            }
            Self::Brush { points, radius } => {
                points
                    .iter()
                    .any(|p| (x - p[0]).powi(2) + (y - p[1]).powi(2) <= radius.powi(2))
                    || points.windows(2).any(|p| {
                        let dx = p[1][0] - p[0][0];
                        let dy = p[1][1] - p[0][1];
                        let t = (((x - p[0][0]) * dx + (y - p[0][1]) * dy)
                            / (dx * dx + dy * dy).max(f32::EPSILON))
                        .clamp(0.0, 1.0);
                        (x - p[0][0] - t * dx).powi(2) + (y - p[0][1] - t * dy).powi(2)
                            <= radius.powi(2)
                    })
            }
            Self::Polygon { points } => {
                let mut inside = false;
                if points.len() < 3 {
                    return false;
                }
                let mut previous = points[points.len() - 1];
                for p in points {
                    if (p[1] > y) != (previous[1] > y)
                        && x < (previous[0] - p[0]) * (y - p[1]) / (previous[1] - p[1]) + p[0]
                    {
                        inside = !inside;
                    }
                    previous = *p;
                }
                inside
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EditRecord {
    pub operation: String,
    pub model: String,
    pub source_sha256: String,
    pub parent_sha256: String,
    pub output_sha256: String,
    pub mask: std::path::PathBuf,
    pub regions: Vec<MaskRegion>,
    pub timestamp: u64,
    #[serde(default)]
    pub dimensions: Option<[u32; 2]>,
    #[serde(default)]
    pub crop: Option<CropRect>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn masks_cover_reversed_rectangles_brush_segments_and_polygon_interior() {
        let rect = MaskRegion::Rectangle {
            start: [40.0, 50.0],
            end: [10.0, 20.0],
        };
        assert!(rect.contains(20.0, 30.0));
        assert!(!rect.contains(5.0, 30.0));
        let brush = MaskRegion::Brush {
            points: vec![[10.0, 10.0], [40.0, 10.0]],
            radius: 3.0,
        };
        assert!(brush.contains(25.0, 12.0));
        assert!(!brush.contains(25.0, 14.0));
        let polygon = MaskRegion::Polygon {
            points: vec![[0.0, 0.0], [20.0, 0.0], [0.0, 20.0]],
        };
        assert!(polygon.contains(5.0, 5.0));
        assert!(!polygon.contains(15.0, 15.0));
    }
}
