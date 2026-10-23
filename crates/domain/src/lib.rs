#![forbid(unsafe_code)]
//! Pure business rules; no UI, database, HTTP or filesystem adapters.
pub mod catalog;
pub mod dedup;
pub mod editing;
pub mod ingest;
pub mod media;
pub mod metadata;
pub mod review;
pub mod tags;
