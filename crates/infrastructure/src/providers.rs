use bdm_domain::ingest::SourcePost;
use bdm_domain::sources::DownloadSource;
use bdm_ports::{CatalogError, IngestError, MediaProvider};

pub struct SourceTagCategories {
    mode: DownloadSource,
    danbooru: Result<crate::danbooru::DanbooruProvider, CatalogError>,
    gelbooru: Result<crate::gelbooru::GelbooruProvider, CatalogError>,
}
impl SourceTagCategories {
    pub fn new(
        mode: DownloadSource,
        danbooru: Result<Option<bdm_ports::SourceCredentials>, CatalogError>,
        gelbooru: Result<Option<bdm_ports::SourceCredentials>, CatalogError>,
    ) -> Self {
        Self {
            mode,
            danbooru: danbooru.and_then(crate::danbooru::DanbooruProvider::new),
            gelbooru: gelbooru.and_then(crate::gelbooru::GelbooruProvider::new),
        }
    }
}
impl bdm_ports::TagCategoryProvider for SourceTagCategories {
    fn lookup_categories(
        &self,
        names: &[String],
    ) -> Result<std::collections::BTreeMap<String, u8>, CatalogError> {
        let primary = if self.mode == DownloadSource::Gelbooru {
            Err(CatalogError::Network)
        } else {
            self.danbooru
                .as_ref()
                .map_err(|e| *e)
                .and_then(|provider| provider.lookup_many(names))
        };
        if self.mode == DownloadSource::Gelbooru
            || (self.mode == DownloadSource::Automatic
                && matches!(
                    primary,
                    Err(CatalogError::Network | CatalogError::Timeout | CatalogError::Service)
                ))
        {
            self.gelbooru.as_ref().map_err(|e| *e)?.lookup_many(names)
        } else {
            primary
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use bdm_ports::TagCategoryProvider;
    #[test]
    fn category_fallback_routes_availability_errors_but_keeps_authentication_failures() {
        let names = vec!["synthetic_visual_tag".into()];
        let fallback = SourceTagCategories {
            mode: DownloadSource::Automatic,
            danbooru: Err(CatalogError::Service),
            gelbooru: Err(CatalogError::GelbooruCredentials),
        };
        assert_eq!(
            fallback.lookup_categories(&names),
            Err(CatalogError::GelbooruCredentials)
        );
        let blocked = SourceTagCategories {
            mode: DownloadSource::Automatic,
            danbooru: Err(CatalogError::Authentication),
            gelbooru: Err(CatalogError::GelbooruCredentials),
        };
        assert_eq!(
            blocked.lookup_categories(&names),
            Err(CatalogError::Authentication)
        );
    }
}
