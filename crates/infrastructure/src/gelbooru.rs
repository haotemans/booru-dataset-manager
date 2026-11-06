//! Official Gelbooru DAPI. API credentials never reach image hosts or diagnostic text.
use bdm_domain::{catalog::SourceTag, ingest::SourcePost, sources::SourceProvider};
use bdm_ports::{CatalogError, IngestError, MediaProvider, SourceCredentials, TagProvider};
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT, CONTENT_TYPE},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};

pub struct GelbooruProvider {
    client: Client,
    media_client: Client,
    credentials: Option<SourceCredentials>,
    categories: Mutex<BTreeMap<String, Option<u8>>>,
    cancel: Option<std::sync::Arc<AtomicBool>>,
}

pub fn valid_credentials(credentials: &SourceCredentials) -> bool {
    credentials.username.parse::<u64>().is_ok_and(|id| id > 0)
        && !credentials.api_key.is_empty()
        && credentials.api_key.len() <= 512
        && !credentials
            .api_key
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
}

impl GelbooruProvider {
    pub fn new(credentials: Option<SourceCredentials>) -> Result<Self, CatalogError> {
        if credentials
            .as_ref()
            .is_some_and(|value| !valid_credentials(value))
        {
            return Err(CatalogError::GelbooruCredentials);
        }
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("BooruDatasetManager/0.1")
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| CatalogError::Network)?;
        let media_client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("BooruDatasetManager/0.1")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|_| CatalogError::Network)?;
        Ok(Self {
            client,
            media_client,
            credentials,
            categories: Mutex::new(BTreeMap::new()),
            cancel: None,
        })
    }

    pub fn with_cancel(mut self, cancel: std::sync::Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    fn request(&self, section: &str, params: &[(&str, &str)]) -> reqwest::blocking::RequestBuilder {
        let mut request = self
            .client
            .get("https://gelbooru.com/index.php")
            .header(ACCEPT, "application/json")
            .query(&[
                ("page", "dapi"),
                ("s", section),
                ("q", "index"),
                ("json", "1"),
            ])
            .query(params);
        if let Some(credentials) = &self.credentials {
            request = request.query(&[
                ("api_key", credentials.api_key.as_str()),
                ("user_id", credentials.username.as_str()),
            ]);
        }
        request
    }

    fn json(&self, section: &str, params: &[(&str, &str)]) -> Result<Value, IngestError> {
        if self
            .cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        {
            return Err(IngestError::Cancelled);
        }
        let response = self
            .request(section, params)
            .send()
            .map_err(network_error)?;
        self.check_status(&response)?;
        if !response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(';')
                    .next()
                    .is_some_and(|mime| mime.trim() == "application/json")
            })
        {
            return Err(CatalogError::InvalidResponse.into());
        }
        let value = read_json(response)?;
        std::thread::sleep(Duration::from_millis(350));
        Ok(value)
    }

    fn check_status(&self, response: &Response) -> Result<(), IngestError> {
        if response
            .headers()
            .get("cf-mitigated")
            .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"challenge"))
        {
            return Err(CatalogError::WebsiteVerification.into());
        }
        match response.status().as_u16() {
            200 => Ok(()),
            401 if self.credentials.is_none() => Err(CatalogError::GelbooruCredentials.into()),
            401 => Err(CatalogError::Authentication.into()),
            403 => Err(CatalogError::AccessDenied.into()),
            429 => Err(IngestError::RateLimited(
                response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(300),
            )),
            500..=599 => Err(CatalogError::Service.into()),
            _ => Err(CatalogError::InvalidResponse.into()),
        }
    }

    pub fn verify_credentials(&self) -> Result<(), CatalogError> {
        if self.credentials.is_none() {
            return Err(CatalogError::GelbooruCredentials);
        }
        let value = self
            .json("post", &[("limit", "1"), ("tags", "official_art")])
            .map_err(catalog_error)?;
        rows(&value, "post").map(|_| ())
    }

    /// Prefix candidates are suggestions, never an alias/identity assertion.
    pub fn suggest_tags(&self, prefix: &str, category: u8) -> Result<Vec<SourceTag>, CatalogError> {
        if prefix.len() < 2
            || prefix.len() > 512
            || prefix.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Ok(Vec::new());
        }
        let pattern = format!("{prefix}%");
        let value = self
            .json(
                "tag",
                &[
                    ("name_pattern", &pattern),
                    ("limit", "50"),
                    ("orderby", "count"),
                    ("order", "DESC"),
                ],
            )
            .map_err(catalog_error)?;
        Ok(prefix_candidates(decode_tags(&value)?, prefix, category))
    }

    pub fn lookup_many(&self, names: &[String]) -> Result<BTreeMap<String, u8>, CatalogError> {
        if names.is_empty() {
            return Ok(BTreeMap::new());
        }
        if names.len() > 64
            || names.iter().any(|name| {
                name.is_empty()
                    || name.len() > 512
                    || name.chars().any(|c| c.is_control() || c.is_whitespace())
            })
        {
            return Err(CatalogError::InvalidTag);
        }
        let joined = names.join(" ");
        let value = self
            .json("tag", &[("names", &joined), ("limit", "100")])
            .map_err(catalog_error)?;
        Ok(decode_tags(&value)?
            .into_iter()
            .filter(|tag| names.contains(&tag.name))
            .map(|tag| (tag.name, tag.category))
            .collect())
    }

    fn classify(&self, posts: &mut [SourcePost]) -> Result<(), IngestError> {
        let all: BTreeSet<_> = posts
            .iter()
            .flat_map(|p| p.tag_string.split_whitespace().map(str::to_owned))
            .collect();
        let missing: Vec<_> = {
            let cache = self.categories.lock().map_err(|_| CatalogError::Service)?;
            all.into_iter()
                .filter(|name| !cache.contains_key(name))
                .collect()
        };
        for batch in missing.chunks(64) {
            let names = batch.join(" ");
            let value = self.json("tag", &[("names", &names), ("limit", "100")])?;
            let tags = decode_tags(&value)?;
            let mut cache = self.categories.lock().map_err(|_| CatalogError::Service)?;
            for name in batch {
                cache.insert(name.clone(), None);
            }
            for tag in tags {
                if batch.contains(&tag.name) {
                    cache.insert(tag.name, Some(tag.category));
                }
            }
        }
        let cache = self.categories.lock().map_err(|_| CatalogError::Service)?;
        for post in posts {
            let mut typed: BTreeMap<u8, Vec<&str>> = BTreeMap::new();
            for tag in post.tag_string.split_whitespace() {
                if let Some(Some(category)) = cache.get(tag) {
                    typed.entry(*category).or_default().push(tag);
                }
            }
            post.tag_string_general = typed.remove(&0).unwrap_or_default().join(" ");
            post.tag_string_artist = typed.remove(&1).unwrap_or_default().join(" ");
            post.tag_string_copyright = typed.remove(&3).unwrap_or_default().join(" ");
            post.tag_string_character = typed.remove(&4).unwrap_or_default().join(" ");
            post.tag_string_meta = typed.remove(&5).unwrap_or_default().join(" ");
        }
        Ok(())
    }
}

fn network_error(error: reqwest::Error) -> IngestError {
    if error.is_timeout() {
        CatalogError::Timeout.into()
    } else {
        CatalogError::Network.into()
    }
}
fn catalog_error(error: IngestError) -> CatalogError {
    match error {
        IngestError::Catalog(error) => error,
        IngestError::RateLimited(_) => CatalogError::RateLimited,
        _ => CatalogError::InvalidResponse,
    }
}
fn read_json(response: Response) -> Result<Value, IngestError> {
    const MAX: u64 = 4 * 1024 * 1024;
    if response.content_length().is_some_and(|size| size > MAX) {
        return Err(CatalogError::InvalidResponse.into());
    }
    let mut bytes = Vec::new();
    response
        .take(MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CatalogError::Network)?;
    if bytes.len() as u64 > MAX {
        return Err(CatalogError::InvalidResponse.into());
    }
    serde_json::from_slice(&bytes).map_err(|_| CatalogError::InvalidResponse.into())
}
fn integer(value: Option<&Value>) -> Option<u64> {
    value.and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    })
}
fn rows<'a>(value: &'a Value, key: &str) -> Result<Vec<&'a Value>, CatalogError> {
    if let Some(items) = value.as_array() {
        return Ok(items.iter().collect());
    }
    match value.get(key) {
        Some(Value::Array(items)) => Ok(items.iter().collect()),
        Some(Value::Object(_)) => Ok(vec![&value[key]]),
        None | Some(Value::Null)
            if integer(value.get("@attributes").and_then(|a| a.get("count"))) == Some(0) =>
        {
            Ok(Vec::new())
        }
        _ => Err(CatalogError::InvalidResponse),
    }
}
fn decode_tags(value: &Value) -> Result<Vec<SourceTag>, CatalogError> {
    rows(value, "tag")?
        .into_iter()
        .map(|row| {
            let id = integer(row.get("id"))
                .filter(|id| *id > 0 && *id <= i64::MAX as u64)
                .ok_or(CatalogError::InvalidResponse)? as i64;
            let name = row
                .get("name")
                .and_then(Value::as_str)
                .ok_or(CatalogError::InvalidResponse)?
                .to_owned();
            let category = integer(row.get("type"))
                .filter(|category| *category <= u8::MAX as u64)
                .ok_or(CatalogError::InvalidResponse)? as u8;
            let post_count = integer(row.get("count"))
                .filter(|count| *count <= i64::MAX as u64)
                .ok_or(CatalogError::InvalidResponse)? as i64;
            if name.is_empty()
                || name.len() > 512
                || name.chars().any(|c| c.is_control() || c.is_whitespace())
            {
                return Err(CatalogError::InvalidResponse);
            }
            Ok(SourceTag {
                provider: SourceProvider::Gelbooru,
                id,
                name,
                category,
                post_count,
            })
        })
        .collect()
}

fn prefix_candidates(tags: Vec<SourceTag>, prefix: &str, category: u8) -> Vec<SourceTag> {
    tags.into_iter()
        .filter(|tag| {
            tag.category == category && tag.name.starts_with(prefix) && tag.name != prefix
        })
        .take(6)
        .collect()
}
fn decode_posts(value: &Value) -> Result<Vec<SourcePost>, CatalogError> {
    let rows = rows(value, "post")?;
    if rows.len() > 100 {
        return Err(CatalogError::InvalidResponse);
    }
    rows.into_iter()
        .map(|row| {
            let id = integer(row.get("id"))
                .filter(|id| *id > 0 && *id <= i64::MAX as u64)
                .ok_or(CatalogError::InvalidResponse)? as i64;
            let file_url = row
                .get("file_url")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
                .map(str::to_owned);
            let file_ext = file_url
                .as_deref()
                .and_then(|url| url.rsplit('.').next())
                .unwrap_or("")
                .to_lowercase();
            let md5 = row
                .get("md5")
                .and_then(Value::as_str)
                .ok_or(CatalogError::InvalidResponse)?
                .to_lowercase();
            if md5.len() != 32 || !md5.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(CatalogError::InvalidResponse);
            }
            let width = integer(row.get("width"))
                .filter(|v| *v <= u32::MAX as u64)
                .ok_or(CatalogError::InvalidResponse)? as u32;
            let height = integer(row.get("height"))
                .filter(|v| *v <= u32::MAX as u64)
                .ok_or(CatalogError::InvalidResponse)? as u32;
            let tags = row
                .get("tags")
                .and_then(Value::as_str)
                .ok_or(CatalogError::InvalidResponse)?
                .to_owned();
            let rating = row
                .get("rating")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            Ok(SourcePost {
                provider: SourceProvider::Gelbooru,
                id,
                md5,
                file_url,
                file_ext,
                file_size: integer(row.get("file_size")).unwrap_or(0),
                image_width: width,
                image_height: height,
                tag_string: tags,
                tag_string_artist: String::new(),
                tag_string_character: String::new(),
                tag_string_general: String::new(),
                tag_string_copyright: String::new(),
                tag_string_meta: String::new(),
                rating,
                is_deleted: false,
            })
        })
        .collect()
}

fn media_url(value: &str) -> Result<reqwest::Url, IngestError> {
    let url = reqwest::Url::parse(value).map_err(|_| IngestError::InvalidUrl)?;
    let valid_host = url.host_str().is_some_and(|host| {
        host == "gelbooru.com"
            || host.strip_suffix(".gelbooru.com").is_some_and(|sub| {
                sub == "img"
                    || sub
                        .strip_prefix("img")
                        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            })
    });
    if url.scheme() != "https"
        || !valid_host
        || url.port().is_some_and(|p| p != 443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/images/")
    {
        return Err(IngestError::InvalidUrl);
    }
    Ok(url)
}
impl TagProvider for GelbooruProvider {
    fn lookup(&self, name: &str) -> Result<Option<SourceTag>, CatalogError> {
        let value = self
            .json("tag", &[("name", name), ("limit", "100")])
            .map_err(catalog_error)?;
        Ok(decode_tags(&value)?
            .into_iter()
            .find(|tag| tag.name == name))
    }
}
impl MediaProvider for GelbooruProvider {
    fn posts(&self, tag: &str, before: Option<i64>) -> Result<Vec<SourcePost>, IngestError> {
        let query = before.map_or_else(
            || format!("{tag} sort:id:desc"),
            |id| format!("{tag} id:<{id} sort:id:desc"),
        );
        let value = self.json("post", &[("tags", &query), ("limit", "100"), ("pid", "0")])?;
        let mut posts = decode_posts(&value)?;
        self.classify(&mut posts)?;
        Ok(posts)
    }
    fn download(
        &self,
        post: &SourcePost,
        path: &std::path::Path,
        cancel: &AtomicBool,
    ) -> Result<(), IngestError> {
        if post.provider != SourceProvider::Gelbooru {
            return Err(IngestError::InvalidUrl);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        let url = media_url(post.file_url.as_deref().ok_or(IngestError::InvalidUrl)?)?;
        let mut response = self.media_client.get(url).send().map_err(network_error)?;
        self.check_status(&response)?;
        if response.content_length().is_some_and(|size| {
            size > 64 * 1024 * 1024 || (post.file_size != 0 && size != post.file_size)
        }) {
            return Err(IngestError::InvalidMedia);
        }
        let mut file = std::fs::File::create(path).map_err(|_| IngestError::Storage)?;
        let mut count = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(IngestError::Cancelled);
            }
            let size = response
                .read(&mut buffer)
                .map_err(|_| CatalogError::Network)?;
            if size == 0 {
                break;
            }
            count += size as u64;
            if count > 64 * 1024 * 1024 || (post.file_size != 0 && count > post.file_size) {
                return Err(IngestError::InvalidMedia);
            }
            file.write_all(&buffer[..size])
                .map_err(|_| IngestError::Storage)?;
        }
        if count == 0 || (post.file_size != 0 && count != post.file_size) {
            return Err(IngestError::Integrity);
        }
        file.sync_all().map_err(|_| IngestError::Storage)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prefix_candidates_filter_wildcards_and_categories_without_claiming_aliases() {
        let tags = decode_tags(&json!({"tag": [
            {"id":1,"name":"synthetic_ore_p","type":1,"count":11},
            {"id":2,"name":"synthetic_ore_character","type":4,"count":12},
            {"id":3,"name":"syntheticXore_p","type":1,"count":13},
            {"id":4,"name":"synthetic_ore","type":1,"count":14}
        ]}))
        .unwrap();
        let candidates = prefix_candidates(tags, "synthetic_ore", 1);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "synthetic_ore_p");
        assert_eq!(candidates[0].provider, SourceProvider::Gelbooru);
    }

    #[test]
    fn api_shapes_preserve_source_and_allow_unknown_size_without_weakening_hash_validation() {
        let row = json!({"id":"42","md5":"0123456789abcdef0123456789abcdef","file_url":"https://img3.gelbooru.com/images/ab/cd/fixture.png","width":1200,"height":"900","tags":"official_art zenless_zone_zero","rating":"general"});
        for value in [
            json!([row.clone()]),
            json!({"post":[row.clone()]}),
            json!({"post":row}),
        ] {
            let post = decode_posts(&value).unwrap().remove(0);
            assert_eq!(post.provider, SourceProvider::Gelbooru);
            assert_eq!(post.file_size, 0);
            assert!(post.skip_reason().is_none());
            let mut legacy = post.clone();
            legacy.provider = SourceProvider::Danbooru;
            assert_eq!(legacy.skip_reason(), Some("file_size_limit"));
        }
        assert!(decode_posts(&json!({"@attributes":{"count":"0"}}))
            .unwrap()
            .is_empty());
        assert!(decode_posts(&json!({"error":"synthetic-private-server-message"})).is_err());
        assert!(decode_posts(&json!({"post":[{"id":0}]})).is_err());
        let tags = decode_tags(
            &json!({"tag":[{"id":"4","name":"synthetic_artist","type":"1","count":"10"}]}),
        )
        .unwrap();
        assert_eq!(tags[0].provider, SourceProvider::Gelbooru);
        assert_eq!(tags[0].category, 1);
        assert!(decode_tags(&json!({"tag":[{"id":5,"name":"?","type":0,"count":1}]})).is_ok());
    }

    #[test]
    fn keys_stay_on_fixed_api_origin_and_original_urls_are_restricted() {
        let provider = GelbooruProvider::new(Some(SourceCredentials {
            username: "123".into(),
            api_key: "unit-secret".into(),
        }))
        .unwrap();
        let request = provider
            .request("post", &[("tags", "official_art zenless_zone_zero")])
            .build()
            .unwrap();
        assert_eq!(request.url().host_str(), Some("gelbooru.com"));
        assert!(request
            .url()
            .query_pairs()
            .any(|(name, value)| name == "api_key" && value == "unit-secret"));
        let url = media_url("https://img3.gelbooru.com/images/ab/cd/fixture.png").unwrap();
        let download = provider.media_client.get(url).build().unwrap();
        assert!(download.url().query().is_none());
        assert!(!download
            .headers()
            .contains_key(reqwest::header::AUTHORIZATION));
        for invalid in [
            "http://img3.gelbooru.com/images/x.png",
            "https://gelbooru.com.evil.example/images/x.png",
            "https://user:unit-secret@img3.gelbooru.com/images/x.png",
            "https://img3.gelbooru.com/samples/x.jpg",
            "https://img3.gelbooru.com/images/x.png?api_key=unit-secret",
            "https://localhost/images/x.png",
        ] {
            assert!(media_url(invalid).is_err());
        }
        assert!(!CatalogError::GelbooruCredentials
            .to_string()
            .contains("unit-secret"));
    }
}
