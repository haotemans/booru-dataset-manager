use bdm_domain::ingest::IngestJob;
use bdm_domain::sources::{DownloadSource, SourceProvider};
use bdm_ports::{IngestError, IngestRepository, MediaProvider};
use std::sync::atomic::{AtomicBool, Ordering};

pub fn source_unavailable(error: &IngestError) -> bool {
    matches!(
        error,
        IngestError::Catalog(
            bdm_ports::CatalogError::Network
                | bdm_ports::CatalogError::Timeout
                | bdm_ports::CatalogError::Service
        )
    )
}

struct SourceRouter<'a> {
    metadata: &'a dyn MediaProvider,
    danbooru: &'a dyn MediaProvider,
    gelbooru: &'a dyn MediaProvider,
}
impl MediaProvider for SourceRouter<'_> {
    fn posts(
        &self,
        tag: &str,
        before: Option<i64>,
    ) -> Result<Vec<bdm_domain::ingest::SourcePost>, IngestError> {
        self.metadata.posts(tag, before)
    }
    fn download(
        &self,
        post: &bdm_domain::ingest::SourcePost,
        path: &std::path::Path,
        cancel: &AtomicBool,
    ) -> Result<(), IngestError> {
        match post.provider {
            SourceProvider::Danbooru => self.danbooru.download(post, path, cancel),
            SourceProvider::Gelbooru => self.gelbooru.download(post, path, cancel),
        }
    }
}

/// A persisted source switch has its own cursor; old failures retain their original provider.
pub fn run_with_fallback(
    danbooru: &dyn MediaProvider,
    gelbooru: &dyn MediaProvider,
    repo: &mut dyn IngestRepository,
    job: &mut IngestJob,
    cancel: &AtomicBool,
    mut progress: impl FnMut(&IngestJob),
) -> Result<(), IngestError> {
    let metadata = match job.provider {
        SourceProvider::Danbooru => danbooru,
        SourceProvider::Gelbooru => gelbooru,
    };
    let router = SourceRouter {
        metadata,
        danbooru,
        gelbooru,
    };
    let result = run(&router, repo, job, cancel, &mut progress);
    let no_eligible = result.is_ok()
        && job.progress.imported == 0
        && job.progress.duplicates == 0
        && job.progress.failed == 0;
    if job.provider == SourceProvider::Danbooru
        && job.source_mode == DownloadSource::Automatic
        && !cancel.load(Ordering::Relaxed)
        && (result.as_ref().err().is_some_and(source_unavailable) || no_eligible)
    {
        job.primary_before_id = job.before_id;
        job.provider = SourceProvider::Gelbooru;
        job.before_id = None;
        job.fallback_reason = Some(
            if no_eligible {
                "Danbooru 没有符合下载条件的结果，已切换 Gelbooru。"
            } else {
                "Danbooru 连接／服务异常，有限重试后已切换 Gelbooru。"
            }
            .into(),
        );
        job.error = None;
        job.state = "queued".into();
        repo.save_job(job)?;
        progress(job);
        return run(
            &SourceRouter {
                metadata: gelbooru,
                danbooru,
                gelbooru,
            },
            repo,
            job,
            cancel,
            progress,
        );
    }
    result
}

/// Descending ID cursors avoid numbered-page limits and make explicit resume idempotent.
pub fn run(
    provider: &dyn MediaProvider,
    repo: &mut dyn IngestRepository,
    job: &mut IngestJob,
    cancel: &AtomicBool,
    mut progress: impl FnMut(&IngestJob),
) -> Result<(), IngestError> {
    job.state = "running".into();
    job.error = None;
    repo.save_job(job)?;
    let result = run_pages(provider, repo, job, cancel, &mut progress);
    job.state = match &result {
        Ok(()) if !job.failed_posts.is_empty() => "deferred",
        Ok(()) => "completed",
        Err(IngestError::Cancelled) => "cancelled",
        Err(_) => "failed",
    }
    .into();
    if let Err(error) = &result {
        if job.error.is_none() {
            job.error = Some(error.to_string());
        }
    }
    if result.is_ok() && !job.failed_posts.is_empty() {
        job.error = Some(format!(
            "{} 张图片重试后仍失败，成功图片已保留；继续任务将只补试失败项。",
            job.failed_posts.len()
        ));
    }
    repo.save_job(job)?;
    progress(job);
    result
}

fn run_pages(
    provider: &dyn MediaProvider,
    repo: &mut dyn IngestRepository,
    job: &mut IngestJob,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(&IngestJob),
) -> Result<(), IngestError> {
    // Retry persisted failures only; never revisit successful page items.
    if bdm_domain::catalog::normalize_download_query(&job.tag).as_deref() != Some(&job.tag) {
        return Err(bdm_ports::CatalogError::InvalidQuery.into());
    }
    let pending = job.failed_posts.clone();
    for post in pending {
        if !bdm_domain::catalog::matches_download_query(&job.tag, &post.tag_string) {
            return Err(bdm_ports::CatalogError::InvalidResponse.into());
        }
        let outcome = process_post(provider, repo, job, &post, cancel).inspect_err(|error| {
            job.error = Some(format!("处理源图 {} 时失败：{error}", post.id));
        })?;
        retry(cancel, || {
            repo.record_source_outcome(job.id, &post, outcome)
        })?;
        if outcome != "failed" {
            job.failed_posts
                .retain(|p| p.id != post.id || p.provider != post.provider);
            job.progress.failed = job.progress.failed.saturating_sub(1);
        }
        retry(cancel, || repo.save_job(job))?;
        progress(job);
    }
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        let posts = retry(cancel, || provider.posts(&job.tag, job.before_id))?;
        if posts.is_empty() {
            return Ok(());
        }
        let mut previous = job.before_id.unwrap_or(i64::MAX);
        for post in &posts {
            if post.id <= 0
                || post.provider != job.provider
                || post.id >= previous
                || !bdm_domain::catalog::matches_download_query(&job.tag, &post.tag_string)
            {
                return Err(bdm_ports::CatalogError::InvalidResponse.into());
            }
            previous = post.id;
        }
        for post in posts {
            if cancel.load(Ordering::Relaxed) {
                return Err(IngestError::Cancelled);
            }
            let outcome =
                process_post(provider, repo, job, &post, cancel).inspect_err(|error| {
                    job.error = Some(format!("处理源图 {} 时失败：{error}", post.id));
                })?;
            if outcome == "failed"
                && !job
                    .failed_posts
                    .iter()
                    .any(|p| p.id == post.id && p.provider == post.provider)
            {
                job.failed_posts.push(post.clone());
                job.progress.failed += 1;
            }
            retry(cancel, || {
                repo.record_source_outcome(job.id, &post, outcome)
            })?;
            job.progress.examined += 1;
            job.before_id = Some(post.id);
            retry(cancel, || repo.save_job(job))?;
            progress(job);
        }
    }
}

fn process_post(
    provider: &dyn MediaProvider,
    repo: &mut dyn IngestRepository,
    job: &mut IngestJob,
    post: &bdm_domain::ingest::SourcePost,
    cancel: &AtomicBool,
) -> Result<&'static str, IngestError> {
    let outcome = if let Some(reason) = post.skip_reason_with_minimum(job.minimum_short_side) {
        job.progress.skipped += 1;
        reason
    } else if retry(cancel, || repo.has_source(post, job.collection_id))? {
        job.progress.duplicates += 1;
        "duplicate"
    } else {
        let imported = retry(cancel, || {
            repo.import_with_options(
                post,
                &job.root,
                job.collection_id,
                job.minimum_short_side,
                job.flatten_transparency,
                cancel,
                provider,
            )
        });
        match imported {
            Ok(true) => {
                job.progress.imported += 1;
                "imported"
            }
            Ok(false) => {
                job.progress.duplicates += 1;
                "duplicate"
            }
            Err(IngestError::InvalidMedia) => {
                job.progress.skipped += 1;
                "invalid_media"
            }
            Err(
                IngestError::Catalog(
                    bdm_ports::CatalogError::Network
                    | bdm_ports::CatalogError::Timeout
                    | bdm_ports::CatalogError::Service,
                )
                | IngestError::RateLimited(_)
                | IngestError::Integrity
                | IngestError::InvalidUrl,
            ) => "failed",
            Err(error) => return Err(error),
        }
    };
    Ok(outcome)
}

fn retry<T>(
    cancel: &AtomicBool,
    mut operation: impl FnMut() -> Result<T, IngestError>,
) -> Result<T, IngestError> {
    for attempt in 0..3 {
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        let result = operation();
        let seconds = match &result {
            Err(IngestError::RateLimited(seconds)) => Some(*seconds),
            Err(IngestError::Catalog(
                bdm_ports::CatalogError::Network
                | bdm_ports::CatalogError::Timeout
                | bdm_ports::CatalogError::Service,
            )) => Some(attempt + 1),
            Err(IngestError::DatabaseBusy) => Some(attempt + 1),
            _ => None,
        };
        if attempt == 2 || seconds.is_none() {
            return result;
        }
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(seconds.unwrap_or(0));
        while std::time::Instant::now() < deadline {
            if cancel.load(Ordering::Relaxed) {
                return Err(IngestError::Cancelled);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    unreachable!("bounded attempts return on the final iteration")
}
