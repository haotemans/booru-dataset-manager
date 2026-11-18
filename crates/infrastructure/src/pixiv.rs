//! Pixiv public illustrations, indexed by artist user ID and artwork page.
use bdm_domain::{
    catalog::SourceTag,
    ingest::{IngestJob, SourcePost},
    sources::SourceProvider,
};
use bdm_ports::{CatalogError, IngestError, MediaProvider};
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT, REFERER, RETRY_AFTER},
    Url,
};
use serde::Deserialize;
use std::{
    cell::OnceCell,
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const ORIGIN: &str = "https://www.pixiv.net";
const PAGE_FACTOR: i64 = 10_000;
const MAX_BYTES: u64 = 64 * 1024 * 1024;

pub struct PixivProvider {
    client: Client,
    user_id: i64,
    tag: String,
    max_works: Option<u32>,
    works: OnceCell<Vec<i64>>,
    cancel: Option<std::sync::Arc<AtomicBool>>,
}

#[derive(Deserialize)]
struct Ajax<T> {
    error: bool,
    body: Option<T>,
}

#[derive(Deserialize)]
struct Profile {
    name: String,
}

#[derive(Deserialize, Default)]
struct Works {
    #[serde(default, deserialize_with = "id_map")]
    illusts: BTreeMap<String, serde::de::IgnoredAny>,
    #[serde(default, deserialize_with = "id_map")]
    manga: BTreeMap<String, serde::de::IgnoredAny>,
}

fn id_map<'de, D: serde::Deserializer<'de>>(
    input: D,
) -> Result<BTreeMap<String, serde::de::IgnoredAny>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Ids {
        Map(BTreeMap<String, serde::de::IgnoredAny>),
        Empty(Vec<serde::de::IgnoredAny>),
    }
    match Ids::deserialize(input)? {
        Ids::Map(ids) => Ok(ids),
        Ids::Empty(items) if items.is_empty() => Ok(BTreeMap::new()),
        Ids::Empty(_) => Err(serde::de::Error::custom("expected Pixiv artwork IDs")),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Work {
    id: String,
    user_id: String,
    page_count: usize,
    illust_type: u8,
    x_restrict: u8,
    width: u32,
    height: u32,
    urls: Option<Urls>,
    tags: Option<Tags>,
}

#[derive(Deserialize)]
struct Tags {
    tags: Vec<Tag>,
}
#[derive(Deserialize)]
struct Tag {
    tag: String,
}
#[derive(Deserialize)]
struct Urls {
    original: Option<String>,
}
#[derive(Deserialize)]
struct Page {
    width: u32,
    height: u32,
    urls: Urls,
}

impl PixivProvider {
    pub fn for_job(
        repo: &crate::catalog::SqliteCatalog,
        job: &IngestJob,
        cancel: std::sync::Arc<AtomicBool>,
    ) -> Result<Self, IngestError> {
        let user_id = crate::ingest::storage(repo.connection.query_row(
            "SELECT source_tag_id FROM collections WHERE id=?1 AND provider='pixiv'",
            [job.collection_id],
            |row| row.get(0),
        ))?;
        Ok(Self::new(user_id, job.tag.clone(), job.pixiv_max_works)?.with_cancel(cancel))
    }

    pub fn new(user_id: i64, tag: String, max_works: Option<u32>) -> Result<Self, CatalogError> {
        let client = Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(45))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("BooruDatasetManager/0.1")
            .build()
            .map_err(|_| CatalogError::Network)?;
        Ok(Self {
            client,
            user_id,
            tag,
            max_works,
            works: OnceCell::new(),
            cancel: None,
        })
    }

    fn with_cancel(mut self, cancel: std::sync::Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    fn ajax<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, IngestError> {
        if self
            .cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        {
            return Err(IngestError::Cancelled);
        }
        let response = self
            .client
            .get(format!("{ORIGIN}/ajax/{path}?lang=en"))
            .header(ACCEPT, "application/json")
            .send()
            .map_err(network)?;
        let response = status(response)?;
        let decoded: Ajax<T> = serde_json::from_reader(response.take(4 * 1024 * 1024))
            .map_err(|_| CatalogError::InvalidResponse)?;
        if decoded.error {
            return Err(CatalogError::AccessDenied.into());
        }
        decoded
            .body
            .ok_or_else(|| CatalogError::InvalidResponse.into())
    }

    fn ids(&self) -> Result<&[i64], IngestError> {
        if self.works.get().is_none() {
            let works: Works = self.ajax(&format!("user/{}/profile/all", self.user_id))?;
            let mut ids = works
                .illusts
                .keys()
                .chain(works.manga.keys())
                .map(|id| {
                    id.parse::<i64>()
                        .ok()
                        .filter(|id| *id > 0 && *id < i64::MAX / PAGE_FACTOR)
                        .ok_or(CatalogError::InvalidResponse)
                })
                .collect::<Result<Vec<_>, _>>()?;
            ids.sort_unstable_by(|a, b| b.cmp(a));
            ids.dedup();
            let _ = self.works.set(ids);
        }
        Ok(self.works.get().ok_or(CatalogError::InvalidResponse)?)
    }

    pub fn artist(&self) -> Result<SourceTag, IngestError> {
        let profile: Profile = self.ajax(&format!("user/{}", self.user_id))?;
        let name = profile
            .name
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("_");
        let name = if bdm_domain::catalog::normalize_download_query(&name).as_deref() == Some(&name)
        {
            name
        } else {
            format!("pixiv_{}", self.user_id)
        };
        Ok(SourceTag {
            provider: SourceProvider::Pixiv,
            id: self.user_id,
            category: 1,
            name,
            post_count: self.ids()?.len() as i64,
        })
    }

    fn pages(&self, id: i64) -> Result<Vec<SourcePost>, IngestError> {
        std::thread::sleep(Duration::from_millis(200));
        let work: Work = match self.ajax(&format!("illust/{id}")) {
            Err(IngestError::Catalog(CatalogError::NotFound)) => return Ok(Vec::new()),
            result => result?,
        };
        if work.id.parse::<i64>().ok() != Some(id)
            || work.user_id.parse::<i64>().ok() != Some(self.user_id)
        {
            return Err(CatalogError::InvalidResponse.into());
        }
        if work.x_restrict != 0 || work.illust_type > 1 {
            return Ok(Vec::new());
        }
        if work.page_count == 0 || work.page_count >= PAGE_FACTOR as usize {
            return Err(CatalogError::InvalidResponse.into());
        }
        let pages: Vec<Page> = if work.page_count == 1 {
            vec![Page {
                width: work.width,
                height: work.height,
                urls: work.urls.ok_or(CatalogError::InvalidResponse)?,
            }]
        } else {
            self.ajax(&format!("illust/{id}/pages"))?
        };
        if pages.len() != work.page_count {
            return Err(CatalogError::InvalidResponse.into());
        }
        let mut raw = vec![self.tag.clone()];
        if let Some(tags) = work.tags {
            raw.extend(
                tags.tags
                    .into_iter()
                    .map(|t| t.tag.split_whitespace().collect::<Vec<_>>().join("_")),
            );
        }
        pages
            .into_iter()
            .enumerate()
            .rev()
            .map(|(index, page)| {
                let page_id = id * PAGE_FACTOR + index as i64;
                let original = page.urls.original.ok_or(CatalogError::InvalidResponse)?;
                let (_, ext) = original_url(&original, page_id)?;
                Ok(SourcePost {
                    provider: SourceProvider::Pixiv,
                    id: page_id,
                    md5: String::new(),
                    file_url: Some(original),
                    file_ext: ext,
                    file_size: 0,
                    image_width: page.width,
                    image_height: page.height,
                    tag_string: raw.join(" "),
                    tag_string_artist: self.tag.clone(),
                    tag_string_character: String::new(),
                    tag_string_general: String::new(),
                    tag_string_copyright: String::new(),
                    tag_string_meta: String::new(),
                    rating: "g".into(),
                    is_deleted: false,
                })
            })
            .collect()
    }
}

fn original_url(raw: &str, page_id: i64) -> Result<(Url, String), IngestError> {
    let url = Url::parse(raw).map_err(|_| IngestError::InvalidUrl)?;
    let (stem, ext) = url
        .path()
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .ok_or(IngestError::InvalidUrl)?;
    let (artwork, page) = stem.rsplit_once("_p").ok_or(IngestError::InvalidUrl)?;
    let expected_artwork = (page_id / PAGE_FACTOR).to_string();
    // Pixiv publishes both historic ID-only names and newer ID-digest names.
    let artwork_matches = artwork == expected_artwork
        || artwork.split_once('-').is_some_and(|(id, digest)| {
            id == expected_artwork
                && digest.len() == 32
                && digest.bytes().all(|b| b.is_ascii_hexdigit())
        });
    if url.scheme() != "https"
        || url.host_str() != Some("i.pximg.net")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/img-original/img/")
        || !artwork_matches
        || page != (page_id % PAGE_FACTOR).to_string()
    {
        return Err(IngestError::InvalidUrl);
    }
    let ext = ext.to_ascii_lowercase();
    Ok((url, ext))
}

fn network(error: reqwest::Error) -> IngestError {
    if error.is_timeout() {
        CatalogError::Timeout.into()
    } else {
        CatalogError::Network.into()
    }
}

fn status(response: Response) -> Result<Response, IngestError> {
    match response.status().as_u16() {
        200 => Ok(response),
        401 | 403 => Err(CatalogError::AccessDenied.into()),
        404 => Err(CatalogError::NotFound.into()),
        429 => Err(IngestError::RateLimited(
            response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(30),
        )),
        500..=599 => Err(CatalogError::Service.into()),
        _ => Err(CatalogError::InvalidResponse.into()),
    }
}

impl MediaProvider for PixivProvider {
    fn posts(&self, tag: &str, before: Option<i64>) -> Result<Vec<SourcePost>, IngestError> {
        if self.user_id <= 0 || tag != self.tag {
            return Err(CatalogError::InvalidQuery.into());
        }
        let before = before.unwrap_or(i64::MAX);
        for &id in self
            .ids()?
            .iter()
            .take(self.max_works.unwrap_or(u32::MAX) as usize)
        {
            if id * PAGE_FACTOR >= before {
                continue;
            }
            let pages: Vec<_> = self
                .pages(id)?
                .into_iter()
                .filter(|page| page.id < before)
                .collect();
            if !pages.is_empty() {
                return Ok(pages);
            }
        }
        Ok(Vec::new())
    }

    fn download(
        &self,
        post: &SourcePost,
        path: &Path,
        cancel: &AtomicBool,
    ) -> Result<(), IngestError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        if post.provider != SourceProvider::Pixiv {
            return Err(IngestError::InvalidUrl);
        }
        let (url, ext) = original_url(
            post.file_url.as_deref().ok_or(IngestError::InvalidUrl)?,
            post.id,
        )?;
        if ext != post.file_ext {
            return Err(IngestError::InvalidUrl);
        }
        let response = status(
            self.client
                .get(url)
                .header(
                    REFERER,
                    format!("{ORIGIN}/artworks/{}", post.id / PAGE_FACTOR),
                )
                .send()
                .map_err(network)?,
        )?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BYTES)
        {
            return Err(IngestError::InvalidMedia);
        }
        let mut source = response.take(MAX_BYTES + 1);
        let mut target = File::create(path).map_err(|_| IngestError::Storage)?;
        let mut buffer = [0u8; 32 * 1024];
        let mut total = 0;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(IngestError::Cancelled);
            }
            let count = source
                .read(&mut buffer)
                .map_err(|_| CatalogError::Network)?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > MAX_BYTES {
                return Err(IngestError::InvalidMedia);
            }
            target
                .write_all(&buffer[..count])
                .map_err(|_| IngestError::Storage)?;
        }
        target.sync_all().map_err(|_| IngestError::Storage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixiv_index_accepts_maps_and_empty_arrays() {
        let works: Ajax<Works> = serde_json::from_str(
            r#"{"error":false,"body":{"illusts":{"123":null,"125":{}},"manga":[]}}"#,
        )
        .unwrap();
        let ids = works.body.unwrap();
        assert_eq!(
            ids.illusts.keys().map(String::as_str).collect::<Vec<_>>(),
            ["123", "125"]
        );
        assert!(ids.manga.is_empty());
    }

    #[test]
    fn original_url_is_bound_to_the_exact_artwork_page_and_host() {
        let original = "https://i.pximg.net/img-original/img/2026/01/01/00/00/00/123_p2.png";
        assert_eq!(
            original_url(original, 123 * PAGE_FACTOR + 2).unwrap().1,
            "png"
        );
        assert!(original_url(original, 123 * PAGE_FACTOR + 1).is_err());
        let with_digest = original.replace("123_p2", "123-868c126d5cd6603cb1c1e292f80922e6_p2");
        assert!(original_url(&with_digest, 123 * PAGE_FACTOR + 2).is_ok());
        assert!(original_url(&with_digest, 12 * PAGE_FACTOR + 2).is_err());
        assert!(original_url(
            &original.replace("i.pximg.net", "i.pximg.net.example.org"),
            123 * PAGE_FACTOR + 2
        )
        .is_err());
    }
}
