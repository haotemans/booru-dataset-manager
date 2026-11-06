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
    Gelbooru,
}

impl DownloadSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "自动：Danbooru → Gelbooru",
            Self::Danbooru => "仅 Danbooru",
            Self::Gelbooru => "仅 Gelbooru",
        }
    }
}
