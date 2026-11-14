#![forbid(unsafe_code)]
//! Native credentials, collection SQLite persistence and Danbooru metadata lookup.
pub mod agent;
pub mod ai;
pub mod catalog;
mod coordination;
pub mod credentials;
pub mod danbooru;
pub mod editing;
pub mod export;
pub mod gelbooru;
pub mod ingest;
pub mod providers;
pub mod visual;
#[cfg(test)]
mod workflow_tests;
pub mod workflows;
