use thiserror::Error;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MediaId(pub i64);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}
impl Dimensions {
    pub const MINIMUM_SIDE: u32 = 800;
    pub fn meets_minimum(self) -> bool {
        self.width >= 800 && self.height >= 800
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimensionDecision {
    Accept(Dimensions),
    RejectTooSmall(Dimensions),
    Unknown,
}
pub fn gate_dimensions(d: Option<Dimensions>) -> DimensionDecision {
    match d {
        Some(x) if x.meets_minimum() => DimensionDecision::Accept(x),
        Some(x) => DimensionDecision::RejectTooSmall(x),
        None => DimensionDecision::Unknown,
    }
}

/// Normalized original-media type, established from provider metadata or file decoding.
/// A thumbnail, filename extension or the first decoded frame is not proof of a static image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    StaticImage,
    Gif,
    AnimatedImage,
    Video,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaTypeDecision {
    Accept,
    SkipGif,
    SkipAnimation,
    SkipVideo,
    Unknown,
}

/// Default acquisition policy: only confirmed static images are eligible.
pub fn gate_media_type(media_type: MediaType) -> MediaTypeDecision {
    match media_type {
        MediaType::StaticImage => MediaTypeDecision::Accept,
        MediaType::Gif => MediaTypeDecision::SkipGif,
        MediaType::AnimatedImage => MediaTypeDecision::SkipAnimation,
        MediaType::Video => MediaTypeDecision::SkipVideo,
        MediaType::Unknown => MediaTypeDecision::Unknown,
    }
}
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MediaError {
    #[error("dimensions unknown; review or explicit override required")]
    UnknownDimensions,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minimum_is_inclusive_for_both_dimensions() {
        assert!(Dimensions {
            width: 800,
            height: 800
        }
        .meets_minimum());
        assert!(!Dimensions {
            width: 799,
            height: 1000
        }
        .meets_minimum());
        assert!(!Dimensions {
            width: 1000,
            height: 799
        }
        .meets_minimum());
    }
    #[test]
    fn unknown_is_not_accepted() {
        assert_eq!(gate_dimensions(None), DimensionDecision::Unknown);
    }

    #[test]
    fn only_confirmed_static_images_pass_the_default_type_gate() {
        assert_eq!(
            gate_media_type(MediaType::StaticImage),
            MediaTypeDecision::Accept
        );
        for (media_type, expected) in [
            (MediaType::Gif, MediaTypeDecision::SkipGif),
            (MediaType::AnimatedImage, MediaTypeDecision::SkipAnimation),
            (MediaType::Video, MediaTypeDecision::SkipVideo),
            (MediaType::Unknown, MediaTypeDecision::Unknown),
        ] {
            assert_eq!(gate_media_type(media_type), expected);
            assert_ne!(gate_media_type(media_type), MediaTypeDecision::Accept);
        }
    }
}
