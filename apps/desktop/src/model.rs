use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Page {
    #[default]
    Dashboard,
    Artist,
    Character,
    Tags,
    Api,
    Logs,
    Settings,
}
impl Page {
    pub const ALL: [Self; 7] = [
        Self::Dashboard,
        Self::Artist,
        Self::Character,
        Self::Tags,
        Self::Api,
        Self::Logs,
        Self::Settings,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Dashboard => "总览",
            Self::Artist => "画师",
            Self::Character => "角色",
            Self::Tags => "标签管理",
            Self::Api => "API 配置",
            Self::Logs => "运行日志",
            Self::Settings => "系统设置",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Self::Dashboard => "dashboard",
            Self::Artist => "artist",
            Self::Character => "character",
            Self::Tags => "tag",
            Self::Api => "api",
            Self::Logs => "logs",
            Self::Settings => "settings",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GalleryLayout {
    #[default]
    Grid,
    Masonry,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtectedTagFormat {
    #[default]
    Plain,
    At,
    Typed,
}
impl ProtectedTagFormat {
    pub fn artist_label(self) -> &'static str {
        match self {
            Self::Plain => "xxx",
            Self::At => "@xxx",
            Self::Typed => "artist:xxx",
        }
    }
    pub fn character_label(self) -> &'static str {
        match self {
            Self::Plain => "xxx",
            Self::At => "@xxx",
            Self::Typed => "character:xxx",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::At => "at",
            Self::Typed => "typed",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub page: Page,
    pub layout: GalleryLayout,
    pub page_size: usize,
    pub root: String,
    pub model_path: String,
    pub danbooru_username: String,
    pub download_media: bdm_config::DownloadMediaPolicy,
    #[serde(default = "bdm_domain::ingest::default_flatten_transparency")]
    pub flatten_transparency: bool,
    #[serde(default = "bdm_domain::ingest::default_minimum_short_side")]
    pub minimum_short_side: u32,
    pub profiles: Vec<ApiProfile>,
    pub confirm_image_edits: bool,
    pub auto_load_review_original: bool,
    pub similar_image_review: bool,
    pub review_similarity: f32,
    #[serde(default = "default_artist_tag_format")]
    pub artist_tag_format: ProtectedTagFormat,
    #[serde(default)]
    pub character_tag_format: ProtectedTagFormat,
}
fn default_artist_tag_format() -> ProtectedTagFormat {
    ProtectedTagFormat::At
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            page: Page::default(),
            layout: GalleryLayout::default(),
            page_size: 50,
            root: String::new(),
            model_path: String::new(),
            danbooru_username: String::new(),
            download_media: bdm_config::DownloadMediaPolicy::default(),
            flatten_transparency: bdm_domain::ingest::default_flatten_transparency(),
            minimum_short_side: bdm_domain::ingest::default_minimum_short_side(),
            profiles: Vec::new(),
            confirm_image_edits: true,
            auto_load_review_original: false,
            similar_image_review: true,
            review_similarity: 0.95,
            artist_tag_format: default_artist_tag_format(),
            character_tag_format: ProtectedTagFormat::Plain,
        }
    }
}
impl Preferences {
    pub fn normalize(&mut self) {
        if !self.review_similarity.is_finite() {
            self.review_similarity = 0.95;
        }
        self.review_similarity = self.review_similarity.clamp(0.85, 0.995);
        if !bdm_domain::ingest::valid_minimum_short_side(self.minimum_short_side) {
            self.minimum_short_side = bdm_domain::ingest::default_minimum_short_side();
        }
        if ![20, 50, 100].contains(&self.page_size) {
            self.page_size = 50;
        }
        let mut ids = BTreeSet::new();
        for profile in &mut self.profiles {
            if profile.id.is_empty()
                || profile.id.len() > 128
                || !profile
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                || ids.contains(&profile.id)
            {
                profile.id = new_profile_id();
            }
            ids.insert(profile.id.clone());
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiProfile {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    pub model: String,
    pub concurrency: u32,
    pub group: String,
    pub request: bdm_config::AiRequestSettings,
}

pub fn new_profile_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{time:x}-{:x}-{:x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
impl ApiProfile {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.name.trim().is_empty() {
            return Err("请填写配置名称。");
        }
        let endpoint =
            url::Url::parse(self.endpoint.trim()).map_err(|_| "请填写有效的服务地址。")?;
        if endpoint.host_str().is_none() || !matches!(endpoint.scheme(), "http" | "https") {
            return Err("请使用有效的 HTTP 或 HTTPS 服务地址。");
        }
        if !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err("地址中不能包含凭据、查询参数或片段，请仅填写服务基础地址。");
        }
        if self.model.trim().is_empty() {
            return Err("请填写模型名称。");
        }
        if !(1..=16).contains(&self.concurrency) {
            return Err("最大并发必须在 1–16 之间。");
        }
        self.request.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    pub id: usize,
    pub tag: String,
    pub api_count: Option<usize>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    pub id: usize,
    pub artist: Option<usize>,
    pub character: Option<usize>,
    pub tags: Vec<String>,
    pub keep: bool,
    pub optimized: bool,
    pub caption: bool,
    pub width: usize,
    pub height: usize,
    pub rating: u8,
}
#[derive(Default)]
pub struct Library {
    pub artists: Vec<Collection>,
    pub characters: Vec<Collection>,
    pub media: Vec<Media>,
}
impl Library {
    pub fn demo() -> Self {
        let artists = [
            "sample_artist_a",
            "sample_artist_b",
            "sample_artist_c",
            "sample_artist_d",
            "sample_artist_e",
            "sample_artist_f",
        ]
        .iter()
        .enumerate()
        .map(|(id, tag)| Collection {
            id,
            tag: tag.to_string(),
            api_count: Some(120 + id * 39),
        })
        .collect();
        let characters = [
            "sample_character_a",
            "sample_character_b",
            "sample_character_c",
            "sample_character_d",
            "sample_character_e",
            "sample_character_f",
        ]
        .iter()
        .enumerate()
        .map(|(id, tag)| Collection {
            id,
            tag: tag.to_string(),
            api_count: Some(248 + id * 173),
        })
        .collect();
        let media = (0..24)
            .map(|id| Media {
                id,
                artist: Some(id % 6),
                character: Some((id / 2) % 6),
                tags: vec![
                    "scenery".into(),
                    if id % 2 == 0 { "mountain" } else { "ocean" }.into(),
                    if id % 3 == 0 { "sunset" } else { "sky" }.into(),
                ],
                keep: id % 4 != 0,
                optimized: id % 3 != 0,
                caption: id % 2 == 0,
                width: 1200 + (id % 3) * 400,
                height: 1000 + (id % 4) * 300,
                rating: (id % 5 + 1) as u8,
            })
            .collect();
        Self {
            artists,
            characters,
            media,
        }
    }
    pub fn collections(&self, page: Page) -> &[Collection] {
        if page == Page::Artist {
            &self.artists
        } else {
            &self.characters
        }
    }
    pub fn associated(&self, page: Page, ids: &BTreeSet<usize>) -> Vec<&Media> {
        self.media
            .iter()
            .filter(|m| {
                let association = if page == Page::Artist {
                    m.artist
                } else {
                    m.character
                };
                association.is_some_and(|id| ids.contains(&id))
            })
            .collect()
    }
    pub fn coverage(&self) -> [usize; 4] {
        [
            self.media.len(),
            self.media.iter().filter(|m| m.optimized).count(),
            self.media.iter().filter(|m| m.caption).count(),
            self.media
                .iter()
                .filter(|m| m.optimized && m.caption)
                .count(),
        ]
    }
}

#[derive(Clone)]
pub struct LogEntry {
    pub time: String,
    pub level: &'static str,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_and_duplicate_profiles_get_distinct_stable_credential_ids() {
        let mut prefs = Preferences {
            profiles: vec![
                ApiProfile::default(),
                ApiProfile::default(),
                ApiProfile {
                    id: "existing-id".into(),
                    ..Default::default()
                },
                ApiProfile {
                    id: "existing-id".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        prefs.normalize();
        let ids: Vec<_> = prefs.profiles.iter().map(|p| p.id.clone()).collect();
        assert!(ids.iter().all(|id| !id.is_empty()));
        assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), 4);
        assert_eq!(ids[2], "existing-id");
        prefs.normalize();
        assert_eq!(
            ids,
            prefs
                .profiles
                .iter()
                .map(|p| p.id.clone())
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn complete_coverage_is_intersection() {
        assert_eq!(Library::demo().coverage(), [24, 16, 12, 8]);
    }
    #[test]
    fn context_scope_is_independent_of_display_filters() {
        let library = Library::demo();
        let scope = library.associated(Page::Artist, &BTreeSet::from([0]));
        assert_eq!(scope.len(), 4);
        assert!(scope.iter().all(|m| m.artist == Some(0)));
    }
    #[test]
    fn accepts_http_and_https_but_rejects_credentials_and_invalid_endpoints() {
        let mut profile = ApiProfile {
            name: "local".into(),
            endpoint: "http://localhost:8000/v1".into(),
            model: "model".into(),
            concurrency: 2,
            group: String::new(),
            ..Default::default()
        };
        assert!(profile.validate().is_ok());
        for endpoint in [
            "http://example.com/v1",
            "http://192.168.1.10:8080/v1",
            "https://example.com/v1",
        ] {
            profile.endpoint = endpoint.into();
            assert!(profile.validate().is_ok());
        }
        for endpoint in [
            "ftp://example.com",
            "https://user:secret@example.com",
            "https://example.com?key=secret",
            "https://",
            "http://localhost:8000@remote.example.com/v1",
        ] {
            profile.endpoint = endpoint.into();
            assert!(profile.validate().is_err());
        }
    }
}
