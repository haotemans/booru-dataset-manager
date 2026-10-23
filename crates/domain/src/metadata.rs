#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextArtifactKind {
    RawTags,
    OptimizedTags,
    Caption,
}
impl TextArtifactKind {
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::RawTags => ".raw_tags.txt",
            Self::OptimizedTags => ".tags.txt",
            Self::Caption => ".txt",
        }
    }
    pub const fn is_provenance(self) -> bool {
        matches!(self, Self::RawTags)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingState {
    NeverProcessed,
    InProgress,
    Failed,
    Complete,
    NeedsReview,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_tags_are_provenance() {
        assert!(TextArtifactKind::RawTags.is_provenance());
        assert!(!TextArtifactKind::OptimizedTags.is_provenance());
    }
}
