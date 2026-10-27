//! Fixed-origin metadata and bounded original-media transfer; credentials stay on API origin.
use bdm_domain::catalog::SourceTag;
use bdm_ports::{CatalogError, IngestError, MediaProvider, SourceCredentials, TagProvider};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, ACCEPT, CONTENT_TYPE};
use serde::Deserialize;
use std::{io::Read, time::Duration};

const RESPONSE_LIMIT: u64 = 64 * 1024;

pub struct DanbooruProvider {
    client: Client,
    credentials: Option<SourceCredentials>,
    media_client: Client,
}

impl DanbooruProvider {
    pub fn new(credentials: Option<SourceCredentials>) -> Result<Self, CatalogError> {
        let client = Client::builder()
            .user_agent("BooruDatasetManager/0.1")
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .build()
            .map_err(|_| CatalogError::Network)?;
        Ok(Self {
            media_client: Client::builder()
                .user_agent("BooruDatasetManager/0.1")
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(90))
                .build()
                .map_err(|_| CatalogError::Network)?,
            client,
            credentials,
        })
    }

    fn request(&self, path: &str) -> reqwest::blocking::RequestBuilder {
        let request = self
            .client
            .get(format!("https://danbooru.donmai.us/{path}"))
            .header(ACCEPT, "application/json");
        if let Some(credentials) = &self.credentials {
            request.basic_auth(&credentials.username, Some(&credentials.api_key))
        } else {
            request
        }
    }

    fn send(request: reqwest::blocking::RequestBuilder) -> Result<Vec<u8>, CatalogError> {
        let response = request.send().map_err(|error| {
            if error.is_timeout() {
                CatalogError::Timeout
            } else {
                CatalogError::Network
            }
        })?;
        status_error(response.status().as_u16(), response.headers())?;
        if response
            .content_length()
            .is_some_and(|length| length > RESPONSE_LIMIT)
        {
            return Err(CatalogError::InvalidResponse);
        }
        read_bounded(response)
    }

    /// Resolve a bounded set of exact Danbooru tags in one request.
    /// This lets AI-only general tags be verified without N separate network calls.
    pub fn lookup_many(
        &self,
        names: &[String],
    ) -> Result<std::collections::BTreeMap<String, u8>, CatalogError> {
        if names.is_empty() {
            return Ok(std::collections::BTreeMap::new());
        }
        if names.len() > 32
            || names.iter().any(|name| {
                name.is_empty()
                    || name.len() > 256
                    || name.contains(',')
                    || name.chars().any(char::is_whitespace)
            })
        {
            return Err(CatalogError::InvalidTag);
        }
        let query = names.join(",");
        let bytes = Self::send(self.request("tags.json").query(&[
            ("search[name]", query.as_str()),
            ("limit", "64"),
            ("only", "id,name,category,post_count"),
        ]))?;
        decode_tag_categories(&bytes, names)
    }

    pub fn verify_credentials(&self) -> Result<(), CatalogError> {
        let credentials = self.credentials.as_ref().ok_or(CatalogError::Credentials)?;
        let bytes = Self::send(self.request("profile.json").query(&[("only", "id,name")]))?;
        verify_profile(&bytes, &credentials.username)
    }
}

fn media_url(value: &str) -> Result<reqwest::Url, IngestError> {
    let url = reqwest::Url::parse(value).map_err(|_| IngestError::InvalidUrl)?;
    if url.scheme() != "https"
        || !matches!(url.host_str(), Some("cdn.donmai.us" | "danbooru.donmai.us"))
        || url.port().is_some_and(|port| port != 443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/original/")
    {
        return Err(IngestError::InvalidUrl);
    }
    Ok(url)
}

impl MediaProvider for DanbooruProvider {
    fn posts(
        &self,
        tag: &str,
        before: Option<i64>,
    ) -> Result<Vec<bdm_domain::ingest::SourcePost>, IngestError> {
        let page = before
            .map(|id| format!("b{id}"))
            .unwrap_or_else(|| "1".into());
        let mut response = self.request("posts.json").query(&[
            ("tags", tag), ("page", &page), ("limit", "100"),
            ("only", "id,md5,file_url,file_ext,file_size,image_width,image_height,tag_string,tag_string_artist,tag_string_character,tag_string_general,tag_string_copyright,tag_string_meta,rating,is_deleted"),
        ]).send().map_err(|error| if error.is_timeout() { CatalogError::Timeout } else { CatalogError::Network })?;
        if response.status().as_u16() == 429 {
            return Err(IngestError::RateLimited(retry_after(response.headers())));
        }
        status_error(response.status().as_u16(), response.headers())?;
        const LIMIT: u64 = 4 * 1024 * 1024;
        if response.content_length().is_some_and(|n| n > LIMIT) {
            return Err(CatalogError::InvalidResponse.into());
        }
        let mut bytes = Vec::new();
        (&mut response)
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| CatalogError::Network)?;
        if bytes.len() as u64 > LIMIT {
            return Err(CatalogError::InvalidResponse.into());
        }
        let posts: Vec<bdm_domain::ingest::SourcePost> =
            serde_json::from_slice(&bytes).map_err(|_| CatalogError::InvalidResponse)?;
        if posts.len() > 100 {
            return Err(CatalogError::InvalidResponse.into());
        }
        std::thread::sleep(Duration::from_millis(250));
        Ok(posts)
    }

    fn download(
        &self,
        post: &bdm_domain::ingest::SourcePost,
        path: &std::path::Path,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<(), IngestError> {
        use std::{io::Write, sync::atomic::Ordering};
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        let url = media_url(post.file_url.as_deref().ok_or(IngestError::InvalidUrl)?)?;
        // This client has no authentication and follows no redirects.
        let mut response = self.media_client.get(url).send().map_err(|error| {
            if error.is_timeout() {
                CatalogError::Timeout
            } else {
                CatalogError::Network
            }
        })?;
        if response
            .headers()
            .get("cf-mitigated")
            .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"challenge"))
        {
            return Err(CatalogError::WebsiteVerification.into());
        }
        match response.status().as_u16() {
            200 => {}
            401 => return Err(CatalogError::Authentication.into()),
            403 => return Err(CatalogError::AccessDenied.into()),
            429 => return Err(IngestError::RateLimited(retry_after(response.headers()))),
            _ => return Err(CatalogError::Service.into()),
        }
        if response
            .content_length()
            .is_some_and(|n| n > 64 * 1024 * 1024 || n != post.file_size)
        {
            return Err(IngestError::InvalidMedia);
        }
        let mut file = std::fs::File::create(path).map_err(|_| IngestError::Storage)?;
        let mut count = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(IngestError::Cancelled);
            }
            let n = response
                .read(&mut buffer)
                .map_err(|_| CatalogError::Network)?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > post.file_size || count > 64 * 1024 * 1024 {
                return Err(IngestError::InvalidMedia);
            }
            file.write_all(&buffer[..n])
                .map_err(|_| IngestError::Storage)?;
        }
        if count != post.file_size {
            return Err(IngestError::Integrity);
        }
        file.sync_all().map_err(|_| IngestError::Storage)?;
        std::thread::sleep(Duration::from_millis(250));
        Ok(())
    }
}

fn retry_after(headers: &HeaderMap) -> u64 {
    headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300)
}

impl DanbooruProvider {
    pub fn character_implications(&self, name: &str) -> Result<Vec<String>, CatalogError> {
        let bytes = Self::send(self.request("tag_implications.json").query(&[
            ("search[antecedent_name]", name),
            ("search[status]", "active"),
            ("limit", "100"),
        ]))?;
        active_implications(&bytes, name)
    }
    pub fn canonical_alias(&self, name: &str) -> Result<Option<String>, CatalogError> {
        let bytes = Self::send(self.request("tag_aliases.json").query(&[
            ("search[antecedent_name]", name),
            ("search[status]", "active"),
            ("limit", "2"),
        ]))?;
        let rows: Vec<serde_json::Value> =
            serde_json::from_slice(&bytes).map_err(|_| CatalogError::InvalidResponse)?;
        Ok(rows
            .into_iter()
            .find(|row| row["status"] == "active" && row["antecedent_name"] == name)
            .and_then(|row| row["consequent_name"].as_str().map(str::to_owned)))
    }
}

fn active_implications(bytes: &[u8], name: &str) -> Result<Vec<String>, CatalogError> {
    #[derive(Deserialize)]
    struct Implication {
        status: String,
        antecedent_name: String,
        consequent_name: String,
    }
    let rows: Vec<Implication> =
        serde_json::from_slice(bytes).map_err(|_| CatalogError::InvalidResponse)?;
    let mut result = Vec::new();
    for row in rows {
        if row.status == "active"
            && row.antecedent_name == name
            && !row.consequent_name.is_empty()
            && !row.consequent_name.chars().any(char::is_whitespace)
            && !result.contains(&row.consequent_name)
        {
            result.push(row.consequent_name);
        }
    }
    Ok(result)
}

#[derive(Deserialize)]
struct ProfilePayload {
    id: i64,
    name: String,
}

fn verify_profile(bytes: &[u8], username: &str) -> Result<(), CatalogError> {
    let profile: ProfilePayload =
        serde_json::from_slice(bytes).map_err(|_| CatalogError::InvalidResponse)?;
    if profile.id <= 0 || !profile.name.eq_ignore_ascii_case(username) {
        return Err(CatalogError::Authentication);
    }
    Ok(())
}

#[derive(Deserialize)]
struct TagPayload {
    id: i64,
    name: String,
    category: u8,
    post_count: i64,
}

fn status_error(status: u16, headers: &HeaderMap) -> Result<(), CatalogError> {
    if headers
        .get("cf-mitigated")
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"challenge"))
    {
        return Err(CatalogError::WebsiteVerification);
    }
    match status {
        200 => {
            let mime = headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                .unwrap_or("")
                .trim();
            if mime.eq_ignore_ascii_case("application/json") {
                Ok(())
            } else {
                Err(CatalogError::InvalidResponse)
            }
        }
        401 => Err(CatalogError::Authentication),
        403 => Err(CatalogError::AccessDenied),
        429 => Err(CatalogError::RateLimited),
        _ => Err(CatalogError::Service),
    }
}

fn read_bounded(reader: impl Read) -> Result<Vec<u8>, CatalogError> {
    let mut bytes = Vec::new();
    reader
        .take(RESPONSE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CatalogError::Network)?;
    if bytes.len() as u64 > RESPONSE_LIMIT {
        return Err(CatalogError::InvalidResponse);
    }
    Ok(bytes)
}

fn decode(reader: impl Read, name: &str) -> Result<Option<SourceTag>, CatalogError> {
    let bytes = read_bounded(reader)?;
    let tags: Vec<TagPayload> =
        serde_json::from_slice(&bytes).map_err(|_| CatalogError::InvalidResponse)?;
    if tags.len() > 1 {
        return Err(CatalogError::InvalidResponse);
    }
    tags.into_iter()
        .next()
        .map(|tag| {
            if tag.id <= 0 || tag.name != name || tag.post_count < 0 {
                return Err(CatalogError::InvalidResponse);
            }
            Ok(SourceTag {
                id: tag.id,
                name: tag.name,
                category: tag.category,
                post_count: tag.post_count,
            })
        })
        .transpose()
}

/// Exact membership is mandatory: wildcard/alias matches may not invent a category.
fn decode_tag_categories(
    bytes: &[u8],
    requested: &[String],
) -> Result<std::collections::BTreeMap<String, u8>, CatalogError> {
    let rows: Vec<TagPayload> =
        serde_json::from_slice(bytes).map_err(|_| CatalogError::InvalidResponse)?;
    let allowed = requested.iter().collect::<std::collections::BTreeSet<_>>();
    if rows.len() > 64 {
        return Err(CatalogError::InvalidResponse);
    }
    let mut resolved = std::collections::BTreeMap::new();
    for row in rows {
        if row.id <= 0 || row.post_count < 0 || !matches!(row.category, 0 | 1 | 3 | 4 | 5) {
            return Err(CatalogError::InvalidResponse);
        }
        if allowed.contains(&row.name) && resolved.insert(row.name, row.category).is_some() {
            return Err(CatalogError::InvalidResponse);
        }
    }
    Ok(resolved)
}

impl TagProvider for DanbooruProvider {
    fn lookup(&self, name: &str) -> Result<Option<SourceTag>, CatalogError> {
        let request = self.request("tags.json").query(&[
            ("search[name]", name),
            ("limit", "2"),
            ("only", "id,name,category,post_count"),
        ]);
        decode(Self::send(request)?.as_slice(), name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_tag_bulk_categories_filter_unrequested_and_preserve_identity_categories() {
        let requested = vec!["blue_hair".into(), "known_artist".into()];
        let records = br#"[
            {"id":11,"name":"blue_hair","category":0,"post_count":350},
            {"id":12,"name":"known_artist","category":1,"post_count":5},
            {"id":13,"name":"unrequested","category":4,"post_count":5}
        ]"#;
        let found = decode_tag_categories(records, &requested).unwrap();
        assert_eq!(found.get("blue_hair"), Some(&0));
        assert_eq!(found.get("known_artist"), Some(&1));
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn automatic_relation_input_requires_active_exact_well_formed_implications() {
        let rows = br#"[
            {"status":"active","antecedent_name":"variant","consequent_name":"base"},
            {"status":"active","antecedent_name":"variant","consequent_name":"base"},
            {"status":"pending","antecedent_name":"variant","consequent_name":"other"},
            {"status":"active","antecedent_name":"unrelated","consequent_name":"other"},
            {"status":"active","antecedent_name":"variant","consequent_name":""}
        ]"#;
        assert_eq!(active_implications(rows, "variant").unwrap(), vec!["base"]);
        assert_eq!(
            active_implications(
                br#"[{"status":"active","antecedent_name":"variant"}]"#,
                "variant"
            ),
            Err(CatalogError::InvalidResponse)
        );
    }

    #[test]
    fn requests_keep_credentials_out_of_urls_and_anonymous_retries() {
        let provider = DanbooruProvider::new(Some(SourceCredentials {
            username: "synthetic_user".into(),
            api_key: "synthetic-secret".into(),
        }))
        .unwrap();
        for path in ["tags.json", "profile.json"] {
            let request = provider.request(path).build().unwrap();
            assert_eq!(request.url().host_str(), Some("danbooru.donmai.us"));
            assert!(!request.url().as_str().contains("synthetic"));
            assert!(request
                .headers()
                .contains_key(reqwest::header::AUTHORIZATION));
        }
        let request = DanbooruProvider::new(None)
            .unwrap()
            .request("tags.json")
            .build()
            .unwrap();
        assert!(!request
            .headers()
            .contains_key(reqwest::header::AUTHORIZATION));
    }

    #[test]
    fn verification_requires_the_authenticated_profile_identity() {
        assert_eq!(
            verify_profile(br#"{"id":42,"name":"Synthetic_User"}"#, "synthetic_user"),
            Ok(())
        );
        for body in [
            br#"{"id":0,"name":"Anonymous"}"#.as_slice(),
            br#"{"id":42,"name":"other_user"}"#.as_slice(),
        ] {
            assert_eq!(
                verify_profile(body, "synthetic_user"),
                Err(CatalogError::Authentication)
            );
        }
        assert_eq!(
            verify_profile(b"[]", "synthetic_user"),
            Err(CatalogError::InvalidResponse)
        );
        assert_eq!(
            DanbooruProvider::new(None).unwrap().verify_credentials(),
            Err(CatalogError::Credentials)
        );
    }

    #[test]
    fn real_shape_empty_mismatch_and_malformed_are_distinct() {
        let json = br#"[{"id":2726116,"name":"phoenix_reffaella","category":4,"post_count":1086}]"#;
        let tag = decode(&json[..], "phoenix_reffaella").unwrap().unwrap();
        assert_eq!(tag.category, 4);
        assert_eq!(tag.post_count, 1086);
        assert_eq!(decode(&b"[]"[..], "missing"), Ok(None));
        for body in [
            "{}",
            "not json",
            "[{\"id\":1}]",
            "[{\"id\":1,\"name\":\"other\",\"category\":4,\"post_count\":0}]",
            "[{\"id\":1,\"name\":\"x\",\"category\":4,\"post_count\":-1}]",
        ] {
            assert_eq!(
                decode(body.as_bytes(), "x"),
                Err(CatalogError::InvalidResponse)
            );
        }
        assert_eq!(
            decode(&vec![b' '; RESPONSE_LIMIT as usize + 1][..], "x"),
            Err(CatalogError::InvalidResponse)
        );
    }

    #[test]
    fn errors_never_include_remote_bodies_or_credentials() {
        for (status, expected) in [
            (401, CatalogError::Authentication),
            (403, CatalogError::AccessDenied),
            (429, CatalogError::RateLimited),
            (302, CatalogError::Service),
            (500, CatalogError::Service),
        ] {
            assert_eq!(status_error(status, &HeaderMap::new()), Err(expected));
            assert!(!expected.to_string().contains("synthetic-secret"));
        }
    }

    #[test]
    fn website_challenge_is_not_reported_as_invalid_credentials_or_missing_tag() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, "text/html; charset=UTF-8".parse().unwrap());
        headers.insert("cf-mitigated", "challenge".parse().unwrap());
        for status in [200, 403, 503] {
            assert_eq!(
                status_error(status, &headers),
                Err(CatalogError::WebsiteVerification)
            );
        }
        headers.remove("cf-mitigated");
        assert_eq!(status_error(403, &headers), Err(CatalogError::AccessDenied));
        assert_eq!(
            status_error(401, &headers),
            Err(CatalogError::Authentication)
        );
        assert_eq!(
            status_error(200, &headers),
            Err(CatalogError::InvalidResponse)
        );
        headers.insert(
            CONTENT_TYPE,
            "application/json; charset=utf-8".parse().unwrap(),
        );
        assert_eq!(status_error(200, &headers), Ok(()));
    }

    #[test]
    #[ignore = "explicit public anonymous read-only network smoke"]
    fn live_public_tag_lookup() {
        let tag = DanbooruProvider::new(None)
            .unwrap()
            .lookup("phoenix_reffaella")
            .unwrap()
            .unwrap();
        assert_eq!(tag.name, "phoenix_reffaella");
        assert_eq!(tag.category, 4);
        assert!(tag.id > 0);
    }

    #[test]
    #[ignore = "Explicit anonymous download and validated ingestion of one public static original into temporary storage"]
    fn live_public_original_ingestion() {
        use bdm_ports::{CollectionRepository, IngestRepository};
        let provider = DanbooruProvider::new(None).unwrap();
        let posts = provider.posts("scenery rating:g", None).unwrap();
        let post = posts
            .into_iter()
            .find(|p| p.skip_reason().is_none())
            .expect("eligible public original");
        let directory = tempfile::tempdir().unwrap();
        let mut repo =
            crate::catalog::SqliteCatalog::open(&directory.path().join("catalog.sqlite3")).unwrap();
        let collection = repo
            .add(
                bdm_domain::catalog::CollectionKind::Character,
                &SourceTag {
                    id: 1,
                    name: "public-smoke-scope".into(),
                    category: 4,
                    post_count: 1,
                },
            )
            .unwrap()
            .record
            .id;
        repo.import(
            &post,
            &directory.path().join("library"),
            collection,
            800,
            &std::sync::atomic::AtomicBool::new(false),
            &provider,
        )
        .unwrap();
        let stored = repo.list_media().unwrap();
        assert_eq!(stored.len(), 1);
        assert!(stored[0].path.is_file());
        assert_eq!(stored[0].post.id, post.id);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "explicit read-only verification of the locally saved Danbooru account"]
    fn live_saved_account_verification() {
        use bdm_ports::SourceCredentialStore;
        let credentials = crate::credentials::NativeCredentialStore::danbooru()
            .load()
            .map_err(|_| CatalogError::Credentials)
            .unwrap()
            .ok_or(CatalogError::Credentials)
            .unwrap();
        DanbooruProvider::new(Some(credentials))
            .unwrap()
            .verify_credentials()
            .unwrap();
    }
}
