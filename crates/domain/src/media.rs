use thiserror::Error;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)] pub struct MediaId(pub i64);
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct Dimensions { pub width: u32, pub height: u32 }
impl Dimensions { pub const MINIMUM_SIDE: u32 = 800; pub fn meets_minimum(self) -> bool { self.width >= 800 && self.height >= 800 } }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum DimensionDecision { Accept(Dimensions), RejectTooSmall(Dimensions), Unknown }
pub fn gate_dimensions(d: Option<Dimensions>) -> DimensionDecision { match d { Some(x) if x.meets_minimum() => DimensionDecision::Accept(x), Some(x) => DimensionDecision::RejectTooSmall(x), None => DimensionDecision::Unknown } }
#[derive(Debug, Error, PartialEq, Eq)] pub enum MediaError { #[error("dimensions unknown; review or explicit override required")] UnknownDimensions }
#[cfg(test)] mod tests { use super::*; #[test] fn minimum_is_inclusive_for_both_dimensions() { assert!(Dimensions{width:800,height:800}.meets_minimum()); assert!(!Dimensions{width:799,height:1000}.meets_minimum()); assert!(!Dimensions{width:1000,height:799}.meets_minimum()); } #[test] fn unknown_is_not_accepted() { assert_eq!(gate_dimensions(None),DimensionDecision::Unknown); } }
