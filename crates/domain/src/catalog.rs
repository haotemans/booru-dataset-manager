#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionKind {
    Artist,
    Character,
}

impl CollectionKind {
    pub fn category(self) -> u8 {
        match self {
            Self::Artist => 1,
            Self::Character => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTag {
    pub id: i64,
    pub name: String,
    pub category: u8,
    pub post_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionRecord {
    pub id: i64,
    pub kind: CollectionKind,
    pub name: String,
    /// Posts provide names but no remote tag ID or total. Keep that absence explicit.
    pub source: Option<SourceTag>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddCollectionResult {
    pub record: CollectionRecord,
    pub created: bool,
}
