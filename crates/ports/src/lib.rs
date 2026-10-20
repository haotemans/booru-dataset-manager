#![forbid(unsafe_code)]
use bdm_domain::media::MediaId;
use thiserror::Error;
#[derive(Debug, Error)] pub enum PortError { #[error("adapter is not implemented")] NotImplemented, #[error("adapter failed: {0}")] Adapter(String) }
pub trait ExactHashLookup { fn find_by_md5(&self, md5_hex:&str)->Result<Option<MediaId>,PortError>; }
