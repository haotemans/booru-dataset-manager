#![forbid(unsafe_code)]
//! Use-case orchestration. UI, SQL, HTTP and model details belong in adapters.
pub fn implementation_status()-> &'static str { "Bootstrap only: ingestion, AI, export, backup and persistence workflows are not implemented." }
