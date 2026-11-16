use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceProvider {
    #[default]
    Danbooru,
    Gelbooru,
}

impl SourceProvider {
    pub fn key(self) -> &'static str {
        match self {
            Self::Danbooru => "danbooru",
            Self::Gelbooru => "gelbooru",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Danbooru => "Danbooru",
            Self::Gelbooru => "Gelbooru",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadSource {
    #[default]
    Automatic,
    Danbooru,
    #[serde(alias = "SafeDanbooru", alias = "safedanbooru", alias = "safebooru")]
    Safebooru,
    Gelbooru,
}

impl DownloadSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "自动：Danbooru → Gelbooru",
            Self::Danbooru => "仅 Danbooru",
            Self::Safebooru => "仅 Safebooru（General）",
            Self::Gelbooru => "仅 Gelbooru",
        }
    }

    pub fn initial_provider(self, saved: SourceProvider) -> SourceProvider {
        match self {
            Self::Automatic => saved,
            Self::Danbooru | Self::Safebooru => SourceProvider::Danbooru,
            Self::Gelbooru => SourceProvider::Gelbooru,
        }
    }

    pub fn danbooru_base_url(self) -> &'static str {
        if self == Self::Safebooru {
            "https://safebooru.donmai.us"
        } else {
            "https://danbooru.donmai.us"
        }
    }

    pub fn active_label(self, provider: SourceProvider) -> &'static str {
        if self == Self::Safebooru && provider == SourceProvider::Danbooru {
            "Safebooru"
        } else {
            provider.label()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn safe_entry_keeps_the_danbooru_identity_namespace_and_persists_its_route() {
        let mode = DownloadSource::Safebooru;
        assert_eq!(
            mode.initial_provider(SourceProvider::Gelbooru),
            SourceProvider::Danbooru
        );
        assert_eq!(mode.danbooru_base_url(), "https://safebooru.donmai.us");
        assert_eq!(mode.active_label(SourceProvider::Danbooru), "Safebooru");
        assert_eq!(
            DownloadSource::Automatic.initial_provider(SourceProvider::Gelbooru),
            SourceProvider::Gelbooru
        );
    }
}
