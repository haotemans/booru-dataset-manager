use bdm_domain::ingest::SourcePost;
use bdm_ports::{CatalogError, IngestError, MediaProvider};
pub struct UnavailableProvider(pub CatalogError);
impl MediaProvider for UnavailableProvider {
    fn posts(&self, _: &str, _: Option<i64>) -> Result<Vec<SourcePost>, IngestError> {
        Err(self.0.into())
    }
    fn download(
        &self,
        _: &SourcePost,
        _: &std::path::Path,
        _: &std::sync::atomic::AtomicBool,
    ) -> Result<(), IngestError> {
        Err(self.0.into())
    }
}
