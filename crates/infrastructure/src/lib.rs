#![forbid(unsafe_code)]
//! Native credentials, collection SQLite persistence and Danbooru metadata lookup.
pub mod ai;
pub mod catalog;
mod coordination;
pub mod credentials;
pub mod danbooru;
pub mod editing;
pub mod export;
pub mod ingest;
pub mod visual;
#[cfg(test)]
mod workflow_tests;
pub mod workflows;
