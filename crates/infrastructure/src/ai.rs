use crate::{catalog::SqliteCatalog, danbooru::DanbooruProvider, ingest::storage, workflows::WorkflowJob};
use bdm_config::AiRequestSettings;
use bdm_domain::tags::TagRules;
use bdm_ports::{ApiCredentials, IngestError};
use reqwest::blocking::Client;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

mod images;
mod identity;
mod protocol;
#[cfg(test)]
mod tests;
use protocol::{BatchResponse, ItemResult};

/// Approval belongs to one explicit dispatch scope and is never saved as a profile preference.
pub struct UploadApproval {
    pub profile_id: String,
    pub endpoint: String,
    pub model: String,
    pub settings: AiRequestSettings,
    pub ids: BTreeSet<i64>,
}

pub struct AiProfile {
    pub id: String,
    pub endpoint: String,
    pub model: String,
    pub group: String,
    pub credentials: Option<ApiCredentials>,
    pub settings: AiRequestSettings,
    pub upload_approval: Option<UploadApproval>,
    /// Formatting is a local presentation choice; these values never affect AI input.
    pub artist_tag_format: String,
    pub character_tag_format: String,
}
#[derive(Debug)]
enum AiError {
    Transport,
    Content,
    ContentFiltered,
    Configuration,
    Image,
    RateLimited(u64),
    Cancelled,
}

impl AiError {
    fn message(&self) -> String {
        match self {
            Self::ContentFiltered => {
                "API 服务端内容过滤拦截（content_filter / refusal），本批未返回有效结果；已成功的结果保留。"
                    .into()
            }
            Self::Content => "API 返回内容为空、格式无效或不符合任务要求，未生成有效结果。".into(),
            Self::Transport => "API 网络连接、请求超时或服务端错误，未生成有效结果。".into(),
            Self::Configuration => "API 认证、权限、模型或服务地址无效；修正配置后继续。".into(),
            Self::Image => "图片读取或压缩失败；未发送该图片。".into(),
            Self::RateLimited(seconds) => {
                format!("API 请求受限；服务要求等待 {seconds} 秒后重试。")
            }
            Self::Cancelled => "任务已暂停。".into(),
        }
    }
}

fn record_failure(job: &mut WorkflowJob, ids: &[i64], message: &str) {
    job.error = Some(message.into());
    for id in ids {
        let history = job.analysis_errors.entry(*id).or_default();
        history.push(message.into());
        if history.len() > 8 {
            history.remove(0);
        }
    }
}
#[derive(Default, Serialize, Deserialize)]
struct RateState {
    deadline: u64,
    active: u32,
    successes: u32,
    last_start: u64,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn format_protected_tag(kind: &str, name: &str, format: &str) -> String {
    match format {
        "typed" => format!("{kind}:{name}"),
        "at" => format!("@{name}"),
        // Empty values preserve the legacy on-disk representation for old direct callers.
        _ if kind == "artist" && format == "legacy" => format!("@artist {name}"),
        _ => name.to_owned(),
    }
}

pub fn format_caption(
    description: &str,
    names: &bdm_domain::tags::NormalizedTags,
    artist_format: &str,
    character_format: &str,
) -> String {
    names
        .artists
        .iter()
        .map(|name| format_protected_tag("artist", name, artist_format))
        .chain(
            names
                .characters
                .iter()
                .map(|name| format_protected_tag("character", name, character_format)),
        )
        .chain(std::iter::once(description.trim().to_owned()))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Recognize only the exact metadata suffix emitted by the previous formatter;
/// leave arbitrary human prose and already formatted captions untouched.
pub fn reformat_legacy_caption(
    content: &str,
    names: &bdm_domain::tags::NormalizedTags,
    artist_format: &str,
    character_format: &str,
) -> Option<String> {
    let mut suffix = String::new();
    if !names.characters.is_empty() {
        suffix.push_str(&format!(" Characters: {}.", names.characters.join(", ")));
    }
    if !names.artists.is_empty() {
        suffix.push_str(&format!(
            " Artist: {}.",
            names
                .artists
                .iter()
                .map(|name| format!("@artist {name}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if suffix.is_empty() {
        return None;
    }
    content
        .trim_end()
        .strip_suffix(&suffix)
        .filter(|description| !description.trim().is_empty())
        .map(|description| format_caption(description, names, artist_format, character_format))
}

#[cfg(test)]
mod formatting_tests {
    use super::{format_caption, format_protected_tag, reformat_legacy_caption};

    #[test]
    fn captions_prefix_artists_then_characters_and_only_migrate_exact_legacy_suffixes() {
        let mut names = bdm_domain::tags::NormalizedTags {
            artists: vec!["tupikeee".into()],
            characters: vec!["phoenix_reffaella".into()],
            ..Default::default()
        };
        let body = "A solo girl sits against a white background.";
        let expected = format!("@tupikeee, phoenix_reffaella, {body}");
        assert_eq!(format_caption(body, &names, "at", "plain"), expected);
        let legacy = format!("{body} Characters: phoenix_reffaella. Artist: @artist tupikeee.");
        assert_eq!(
            reformat_legacy_caption(&legacy, &names, "at", "plain"),
            Some(expected.clone())
        );
        assert!(reformat_legacy_caption(&expected, &names, "at", "plain").is_none());
        assert!(reformat_legacy_caption(
            "Artist: a sign in the background.",
            &names,
            "at",
            "plain"
        )
        .is_none());
        names.artists.push("another_artist".into());
        names.characters.push("another_character".into());
        assert_eq!(format_caption(body, &names, "plain", "typed"), format!("tupikeee, another_artist, character:phoenix_reffaella, character:another_character, {body}"));
        assert_eq!(
            format_caption(body, &Default::default(), "at", "plain"),
            body
        );
    }

    #[test]
    fn protected_name_formats_are_local_and_explicit() {
        assert_eq!(format_protected_tag("artist", "alice", "at"), "@alice");
        assert_eq!(format_protected_tag("artist", "alice", "plain"), "alice");
        assert_eq!(
            format_protected_tag("artist", "alice", "typed"),
            "artist:alice"
        );
        assert_eq!(
            format_protected_tag("character", "alice", "typed"),
            "character:alice"
        );
        assert_eq!(format_protected_tag("character", "alice", "plain"), "alice");
    }
}

fn wait_until(deadline: u64, cancel: &AtomicBool) -> Result<(), AiError> {
    while now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            return Err(AiError::Cancelled);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

fn request(
    client: &Client,
    profile: &AiProfile,
    items: &[Value],
    images: &[images::PreparedImage],
    mode: &str,
    cancel: &AtomicBool,
) -> Result<BatchResponse, AiError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(AiError::Cancelled);
    }
    let mut url =
        reqwest::Url::parse(profile.endpoint.trim()).map_err(|_| AiError::Configuration)?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if (url.scheme() != "https" && !(url.scheme() == "http" && local))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err(AiError::Configuration);
    }
    let path = url.path().trim_end_matches('/');
    if !path.ends_with("/chat/completions") {
        url.set_path(&format!("{path}/chat/completions"));
    }
    let body = protocol::body(profile, items, images, mode)?;
    let bytes = serde_json::to_vec(&body).map_err(|_| AiError::Content)?;
    if bytes.len() > protocol::MAX_REQUEST_BYTES {
        return Err(AiError::Content);
    }
    drop(body);
    let mut request = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(bytes);
    if let Some(credentials) = &profile.credentials {
        if credentials.endpoint.trim_end_matches('/')
            != profile.endpoint.trim().trim_end_matches('/')
        {
            return Err(AiError::Configuration);
        }
        request = request.bearer_auth(&credentials.api_key);
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(AiError::Cancelled);
    }
    let response = request.send().map_err(|_| AiError::Transport)?;
    match response.status().as_u16() {
        400 | 401 | 403 | 404 | 422 => return Err(AiError::Configuration),
        429 => {
            return Err(AiError::RateLimited(
                response
                    .headers()
                    .get("retry-after")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|h| h.parse::<u64>().ok())
                    .unwrap_or(300)
                    .min(3600),
            ))
        }
        500..=599 => return Err(AiError::Transport),
        200 => {}
        _ => return Err(AiError::Content),
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    response
        .take(protocol::MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AiError::Transport)?;
    if bytes.len() > protocol::MAX_RESPONSE_BYTES {
        return Err(AiError::Content);
    }
    let response: Value = serde_json::from_slice(&bytes).map_err(|_| AiError::Content)?;
    if response
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        == Some("content_filter")
        || response
            .pointer("/choices/0/message/refusal")
            .is_some_and(|v| !v.is_null() && v.as_str().is_none_or(|text| !text.trim().is_empty()))
    {
        return Err(AiError::ContentFiltered);
    }
    let content_value = response
        .pointer("/choices/0/message/content")
        .ok_or(AiError::Content)?;
    let content = if let Some(content) = content_value.as_str() {
        content.to_owned()
    } else if content_value.is_object() {
        serde_json::to_string(content_value).map_err(|_| AiError::Content)?
    } else {
        return Err(AiError::Content);
    };
    protocol::parse(
        &content,
        &items.iter().filter_map(|v| v["id"].as_i64()).collect(),
        mode,
    )
}

/// Resolve new model-suggested terms using Danbooru's authoritative category.
fn update_candidate_categories(
    repo: &mut SqliteCatalog,
    rules: &mut TagRules,
    already_checked: &mut BTreeSet<String>,
    candidates: &[String],
) -> Result<(), IngestError> {
    let mut pending = Vec::new();
    for candidate in candidates.iter().take(128) {
        let normalized = candidate.trim().to_lowercase().replace(' ', "_");
        if normalized.is_empty()
            || normalized.len() > 256
            || !normalized.bytes().all(|byte| byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'(' | b')' | b'-'))
        {
            continue;
        }
        let canonical = rules.canonical(&normalized);
        if !rules.categories.contains_key(&canonical)
            && !rules.protected.contains(&canonical)
            && already_checked.insert(canonical.clone())
        {
            pending.push(canonical);
        }
    }
    if pending.is_empty() {
        return Ok(());
    }
    let Ok(provider) = DanbooruProvider::new(None) else {
        return Ok(());
    };
    let mut changed = false;
    for chunk in pending.chunks(32) {
        let Ok(categories) = provider.lookup_many(chunk) else {
            break;
        };
        for (name, category) in categories {
            if matches!(category, 0 | 1 | 3 | 4 | 5)
                && rules.categories.insert(name, category) != Some(category)
            {
                changed = true;
            }
        }
    }
    if changed {
        repo.save_tag_rules(rules)?;
    }
    Ok(())
}

pub fn run(
    repo: &mut SqliteCatalog,
    job: &mut WorkflowJob,
    profile: &AiProfile,
    cancel: &AtomicBool,
    mut report: impl FnMut(&WorkflowJob),
) -> Result<(), IngestError> {
    if let Err(error) = profile.settings.validate() {
        job.state = "paused".into();
        job.error = Some(error.into());
        repo.save_workflow(job)?;
        report(job);
        return Ok(());
    }
    // Optimization always requires a vision-capable profile; text-only mode is retired.
    if !profile.settings.send_images {
        job.state = "paused".into();
        job.error = Some("优化需要发送图片与 Danbooru 元数据，请在 API 配置中开启图片发送。".into());
        repo.save_workflow(job)?;
        report(job);
        return Ok(());
    }
    let send_images = true;
    if send_images
        && !profile.upload_approval.as_ref().is_some_and(|approval| {
            approval.profile_id == profile.id
                && approval.endpoint == profile.endpoint
                && approval.model == profile.model
                && approval.settings == profile.settings
                && job
                    .ids
                    .iter()
                    .filter(|id| !job.accepted.contains(id))
                    .all(|id| approval.ids.contains(id))
        })
    {
        job.state = "awaiting_approval".into();
        job.error = Some("本次图片发送尚未确认，请检查范围并允许发送后继续。".into());
        repo.save_workflow(job)?;
        report(job);
        return Ok(());
    }
    let mut builder = Client::builder();
    if reqwest::Url::parse(&profile.endpoint)
        .ok()
        .is_some_and(|u| matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
    {
        builder = builder.no_proxy();
    }
    let client = builder
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(profile.settings.timeout_seconds))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| IngestError::Storage)?;
    let scope = if profile.group.is_empty() {
        format!("profile:{}", profile.id)
    } else {
        format!("group:{}", profile.group)
    };
    let json: Option<String> = storage(
        repo.connection
            .query_row(
                "SELECT metadata FROM api_rate_state WHERE scope=?1",
                [&scope],
                |r| r.get(0),
            )
            .optional(),
    )?;
    let mut rate: RateState = json
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .map_err(|_| IngestError::Storage)?
        .unwrap_or_else(|| RateState {
            active: 2,
            ..Default::default()
        });
    let all = repo
        .list_media()?
        .into_iter()
        .map(|m| (m.id, m))
        .collect::<BTreeMap<_, _>>();
    let mut rules = repo.tag_rules()?;
    let mut already_checked = BTreeSet::new();
    let artifacts = repo.list_artifacts()?;
    for artifact in artifacts.iter().filter(|a| {
        a.kind == job.mode
            && a.provenance
                == format!(
                    "{}:{}:job-{}:bdm-text-v1",
                    profile.id, profile.model, job.id
                )
    }) {
        job.accepted.insert(artifact.media_id);
    }
    job.state = "running".into();
    job.error = None;
    repo.save_workflow(job)?;
    let mut recovery = job.cursor >= job.ids.len();
    let mut pending = if recovery {
        if !job.recovery.is_empty() {
            std::mem::take(&mut job.recovery)
        } else {
            std::mem::take(&mut job.deferred)
        }
    } else {
        Vec::new()
    };
    let maximum_batch = if send_images { 1 } else { profile.settings.batch_size };
    let mut batch_size = job.fallback.clamp(1, maximum_batch);
    let result: Result<(), IngestError> = (|| {
        loop {
            if cancel.load(Ordering::Relaxed) {
                job.state = "paused".into();
                break;
            }
            if !recovery && job.cursor >= job.ids.len() {
                recovery = true;
                pending = std::mem::take(&mut job.deferred);
                batch_size = maximum_batch;
            }
            if recovery && pending.is_empty() {
                job.state = if job.deferred.is_empty() {
                    "completed"
                } else {
                    "deferred"
                }
                .into();
                break;
            }
            let ids: Vec<i64> = if recovery {
                pending.iter().take(batch_size).copied().collect()
            } else {
                job.ids
                    .iter()
                    .skip(job.cursor)
                    .take(maximum_batch)
                    .copied()
                    .collect()
            };
            let work: Vec<i64> = ids
                .iter()
                .copied()
                .filter(|id| !job.accepted.contains(id))
                .collect();
            if work.is_empty() {
                if recovery {
                    pending.drain(..ids.len());
                } else {
                    job.cursor += ids.len();
                }
                continue;
            }
            let mut constraints = BTreeMap::new();
            let mut input = Vec::new();
            let mut images = Vec::new();
            for id in &work {
                if let Some(media) = all.get(id) {
                    let normalized = bdm_domain::tags::normalize(
                        &media.post.tag_string,
                        &media.post.tag_string_artist,
                        &media.post.tag_string_character,
                        &rules,
                    );
                    // Artist and character names are excluded; unknown/special terms remain local.
                    let paired_txt = if send_images {
                        match std::fs::read_to_string(media.path.with_extension("raw_tags.txt")) {
                            Ok(raw) if raw.len() <= 64 * 1024 && !raw.contains('\0') => {
                                Some(identity::generic_metadata_prior(&raw, &normalized, &rules))
                            }
                            _ => {
                                job.analysis_errors.entry(*id).or_default()
                                    .push("Danbooru 原始元数据 TXT 无法读取或内容无效；未发送图片。".into());
                                continue;
                            }
                        }
                    } else { None };
                    if send_images {
                        match images::prepare(*id, &media.path, &profile.settings, cancel) {
                            Ok(image) => images.push(image),
                            Err(AiError::Cancelled) => break,
                            Err(_) => {
                                job.analysis_errors
                                    .entry(*id)
                                    .or_default()
                                    .push("图片读取或压缩失败；未发送该图片。".into());
                                // Keep a failed image out of this multimodal batch. The
                                // item is deferred below and can be retried independently.
                                continue;
                            }
                        }
                    }
                    input.push(if let Some(txt) = paired_txt {
                        json!({"id":id,"tags":normalized.ordinary,"paired_txt":txt})
                    } else {
                        json!({"id":id,"tags":normalized.ordinary})
                    });
                    constraints.insert(*id, normalized);
                }
            }
            if cancel.load(Ordering::Relaxed) {
                job.state = "paused".into();
                break;
            }
            if input.is_empty() {
                for id in &work {
                    if !job.accepted.contains(id) && !job.deferred.contains(id) {
                        job.deferred.push(*id);
                    }
                }
                if recovery {
                    pending.drain(..ids.len());
                } else {
                    job.cursor += ids.len();
                }
                continue;
            }
            let mut transport_retries = 0;
            let mut rate_retries = 0;
            let response = loop {
                if wait_until(
                    rate.deadline.max(if rate.active == 1 {
                        rate.last_start + 1
                    } else {
                        0
                    }),
                    cancel,
                )
                .is_err()
                {
                    break Err(AiError::Cancelled);
                }
                rate.last_start = now();
                storage(repo.connection.execute(
                    "INSERT OR REPLACE INTO api_rate_state VALUES(?1,?2)",
                    params![scope, storage(serde_json::to_string(&rate))?],
                ))?;
                let response = request(&client, profile, &input, &images, &job.mode, cancel);
                match &response {
                    Err(AiError::RateLimited(seconds)) => {
                        rate.deadline = now() + seconds;
                        rate.active = 1;
                        rate.successes = 0;
                        storage(repo.connection.execute(
                            "INSERT OR REPLACE INTO api_rate_state VALUES(?1,?2)",
                            params![scope, storage(serde_json::to_string(&rate))?],
                        ))?;
                        report(job);
                        if rate_retries >= 1 {
                            break response;
                        }
                        rate_retries += 1;
                    }
                    Err(AiError::Transport)
                        if transport_retries < profile.settings.transport_retries =>
                    {
                        transport_retries += 1;
                        if wait_until(now() + (1 << transport_retries), cancel).is_err() {
                            break Err(AiError::Cancelled);
                        }
                    }
                    _ => break response,
                }
            };
            if matches!(response, Err(AiError::Cancelled)) {
                job.state = "paused".into();
                break;
            }
            if matches!(response, Err(AiError::Configuration)) {
                job.state = "paused".into();
                record_failure(job, &work, &AiError::Configuration.message());
                break;
            }
            // Resolve proposed new generic tags against Danbooru before accepting
            // them. A category lookup failure defers the item rather than silently
            // dropping all novel details or trusting a model-invented identity.
            let mut category_lookup_failed = false;
            if job.mode == "tags" {
                if let Ok(value) = &response {
                    let mut unknown = BTreeSet::new();
                    for row in value.items.values() {
                        if let ItemResult::Tags(tags) = row {
                            for tag in tags {
                                let name = tag.trim().to_lowercase().replace(' ', "_");
                                if name.is_empty()
                                    || name.len() > 256
                                    || name.contains('@')
                                    || name.contains(':')
                                    || name.contains(',')
                                    || name.chars().any(char::is_control)
                                    || rules.protected.contains(&name)
                                {
                                    continue;
                                }
                                let canonical = rules.canonical(&name);
                                if !rules.categories.contains_key(&canonical) {
                                    unknown.insert(canonical);
                                }
                            }
                        }
                    }
                    if !unknown.is_empty() {
                        let proposed = unknown.into_iter().collect::<Vec<_>>();
                        if proposed.len() > 128 {
                            category_lookup_failed = true;
                        } else {
                            match DanbooruProvider::new(None) {
                                Ok(provider) => {
                                    let mut updated = false;
                                    for chunk in proposed.chunks(32) {
                                        match provider.lookup_many(chunk) {
                                            Ok(categories) => {
                                                for (name, category) in categories {
                                                    updated |= rules.categories.insert(name, category) != Some(category);
                                                }
                                            }
                                            Err(_) => {
                                                category_lookup_failed = true;
                                                break;
                                            }
                                        }
                                    }
                                    if updated {
                                        repo.save_tag_rules(&rules)?;
                                    }
                                }
                                Err(_) => category_lookup_failed = true,
                            }
                        }
                    }
                }
            }
            let mut accepted = BTreeSet::new();
            let content_failure =
                category_lookup_failed || matches!(response, Err(AiError::Content | AiError::ContentFiltered));
            if let Ok(ref value) = response {
                for (&id, row) in &value.items {
                    let Some(media) = all.get(&id) else {
                        continue;
                    };
                    let Some(normalized) = constraints.get(&id) else {
                        continue;
                    };
                    let text = if let ItemResult::Tags(tags) = row {
                        if category_lookup_failed {
                            continue;
                        }
                        if tags.len() > 512
                            || tags.iter().any(|t| {
                                t.len() > 256
                                    || t.chars().any(char::is_control)
                                    || t.trim().is_empty()
                            })
                        {
                            continue;
                        }
                        update_candidate_categories(
                            repo,
                            &mut rules,
                            &mut already_checked,
                            tags,
                        )?;
                        let tags = identity::verified_visual_tags(tags, &rules);
                        if tags.is_empty() {
                            // Do not replace the last usable artifact with only rejected identity tags.
                            continue;
                        }
                        let mut merged = normalized
                            .tags
                            .iter()
                            .filter(|tag| !normalized.ordinary.contains(tag))
                            .map(|tag| {
                                if let Some(name) = normalized.artists.iter().find(|name| {
                                    name.as_str() == tag.as_str()
                                        || format!("@artist {name}").as_str() == tag.as_str()
                                }) {
                                    format_protected_tag("artist", name, &profile.artist_tag_format)
                                } else if normalized.characters.iter().any(|name| name == tag) {
                                    format_protected_tag(
                                        "character",
                                        tag,
                                        &profile.character_tag_format,
                                    )
                                } else {
                                    tag.clone()
                                }
                            })
                            .collect::<Vec<_>>();
                        merged.extend(tags.iter().cloned());
                        merged.join(", ")
                    } else {
                        let ItemResult::Caption(caption) = row else {
                            continue;
                        };
                        if caption.is_empty()
                            || caption.len() > 8192
                            || caption
                                .chars()
                                .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
                        {
                            continue;
                        }
                        if identity::caption_mentions_identity(
                            caption,
                            &normalized,
                            &rules,
                            &media.post.tag_string_copyright,
                        ) {
                            continue;
                        }
                        format_caption(
                            caption,
                            &normalized,
                            &profile.artist_tag_format,
                            &profile.character_tag_format,
                        )
                    };
                    repo.write_ai_artifact(
                        media,
                        &job.mode,
                        &text,
                        &format!(
                            "{}:{}:job-{}:bdm-text-v1",
                            profile.id, profile.model, job.id
                        ),
                    )?;
                    job.accepted.insert(id);
                    job.analysis_errors.remove(&id);
                    accepted.insert(id);
                }
            }
            let failed: Vec<_> = work
                .iter()
                .copied()
                .filter(|id| !job.accepted.contains(id))
                .collect();
            if !failed.is_empty() {
                let message = match &response {
                    Err(error) => error.message(),
                    Ok(_) => "API 未返回对应图片的有效结果，或结果未通过本地校验。".into(),
                };
                record_failure(job, &failed, &message);
                repo.audit(
                    "api_batch_failure",
                    &json!({"job_id":job.id,"mode":job.mode,"ids":failed,"reason":message})
                        .to_string(),
                )?;
            } else {
                job.error = None;
            }
            if failed.is_empty() {
                rate.successes += 1;
                if rate.active == 1 && rate.successes >= 5 {
                    rate.active = 2;
                    rate.successes = 0;
                }
            }
            if recovery {
                if !failed.is_empty()
                    && batch_size > 1
                    && !matches!(response, Err(AiError::Transport | AiError::RateLimited(_)))
                {
                    batch_size /= 2;
                    job.fallback = batch_size;
                } else {
                    pending.drain(..ids.len());
                    for id in failed {
                        if !job.deferred.contains(&id) {
                            job.deferred.push(id);
                        }
                    }
                    batch_size = maximum_batch;
                }
            } else {
                for id in failed {
                    if !job.deferred.contains(&id) {
                        job.deferred.push(id);
                    }
                }
                job.cursor += if content_failure && ids.len() == maximum_batch && maximum_batch > 1
                {
                    maximum_batch / 2
                } else {
                    ids.len()
                };
            }
            job.deferred.retain(|id| !job.accepted.contains(id));
            job.recovery = pending.clone();
            storage(repo.connection.execute(
                "INSERT OR REPLACE INTO api_rate_state VALUES(?1,?2)",
                params![scope, storage(serde_json::to_string(&rate))?],
            ))?;
            repo.save_workflow(job)?;
            report(job);
        }
        Ok(())
    })();
    if recovery {
        for id in pending {
            if !job.accepted.contains(&id) && !job.deferred.contains(&id) {
                job.deferred.push(id);
            }
        }
    }
    job.recovery = Vec::new();
    if job.state == "completed" {
        job.error = None;
    } else if !job.deferred.is_empty() && job.error.is_none() {
        job.error = Some(format!(
            "{} 张图片尚未生成成功，可继续任务重试；请查看失败记录。",
            job.deferred.len()
        ));
    }
    if let Err(error) = &result {
        job.state = "failed".into();
        job.error = Some(error.to_string());
    }
    repo.save_workflow(job)?;
    report(job);
    result
}
