use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DuplicateCandidate {
    pub first: i64,
    pub second: i64,
    pub phash_distance: Option<u32>,
    pub cosine: Option<f32>,
    pub model_version: Option<String>,
    pub state: String,
}
impl DuplicateCandidate {
    pub fn dual_confirmed(&self) -> bool {
        self.phash_distance.is_some_and(|d| d <= 2)
            && self
                .cosine
                .is_some_and(|s| s.is_finite() && (0.997..=1.0).contains(&s))
            && self.model_version.as_ref().is_some_and(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SurvivorQuality {
    pub aesthetic: Option<f64>,
    pub pixels: u64,
    pub png: bool,
    pub watermark_free: Option<bool>,
    pub id: i64,
}
pub fn prefer_first(a: SurvivorQuality, b: SurvivorQuality) -> bool {
    if let (Some(x), Some(y)) = (a.aesthetic, b.aesthetic) {
        if x.is_finite() && y.is_finite() && x != y {
            return x > y;
        }
    }
    (
        a.pixels,
        a.png,
        a.watermark_free == Some(true),
        std::cmp::Reverse(a.id),
    ) >= (
        b.pixels,
        b.png,
        b.watermark_free == Some(true),
        std::cmp::Reverse(b.id),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_signals_and_real_model_identity_are_required() {
        let mut pair = DuplicateCandidate {
            first: 1,
            second: 2,
            phash_distance: Some(2),
            cosine: None,
            model_version: None,
            state: "pending".into(),
        };
        assert!(!pair.dual_confirmed());
        pair.cosine = Some(0.997);
        pair.model_version = Some("dinov3-version".into());
        assert!(pair.dual_confirmed());
        pair.cosine = Some(f32::NAN);
        assert!(!pair.dual_confirmed());
    }
    #[test]
    fn unavailable_aesthetic_falls_back_and_watermark_unknown_is_not_free() {
        let a = SurvivorQuality {
            pixels: 1000,
            id: 1,
            ..Default::default()
        };
        let b = SurvivorQuality {
            pixels: 2000,
            id: 2,
            ..Default::default()
        };
        assert!(!prefer_first(a, b));
        let c = SurvivorQuality {
            pixels: 1000,
            watermark_free: Some(true),
            id: 2,
            ..Default::default()
        };
        assert!(!prefer_first(a, c));
        assert!(prefer_first(
            SurvivorQuality {
                aesthetic: Some(9.0),
                ..a
            },
            SurvivorQuality {
                aesthetic: Some(8.0),
                ..b
            }
        ));
    }
}
