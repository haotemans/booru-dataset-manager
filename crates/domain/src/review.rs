#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewStatus {
    Pending,
    Keep,
    Unsatisfactory,
    NeedsReview,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rating(u8);
impl Rating {
    pub fn new(value: u8) -> Option<Self> {
        (1..=5).contains(&value).then_some(Self(value))
    }
    pub const fn value(self) -> u8 {
        self.0
    }
}
