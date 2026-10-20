# Ingestion job state and retry contract

## Job states

`QUEUED → DISCOVERING → FETCHING_METADATA → DIMENSION_GATE → DOWNLOADING → VALIDATING_FILE → HASHING → EXACT_DEDUP_CHECK → IMPORTING → EMBEDDING_PENDING | EMBEDDING → SIMILARITY_SEARCH → REVIEW_REQUIRED | IDENTITY_ASSIGNED → COMPLETED`.

Terminal alternatives: `SKIPPED`, `FAILED_RETRYABLE`, `FAILED_PERMANENT`, `CANCELLED`. Persist stage, attempt count, last error code, retry-after time, source cursor and progress counters. A crash must not make a completed media import appear as a fresh unrelated item.

## Retry classes

- Retryable: transient network failure, provider 429/5xx, temporary disk busy, remote service timeout, transient model runtime failure.
- Permanent until user action: invalid credentials, unsupported media type, policy restriction, malformed source metadata, insufficient resolution, invalid output path.
- Review required: unknown dimensions, ambiguous visual identity, unknown/protected tag conflict, incomplete caption validation.

Use bounded exponential backoff with jitter, honoring provider `Retry-After`. Never retry indefinitely or retry an authentication failure as if it were a network outage. Cancellation stops scheduling new work and cleans temporary resources; already committed imports remain committed and are reported as such.

## Idempotency

A repeated provider record maps to the same source identity key. Repeated download of the same bytes must not create duplicate media. Retried embedding writes are keyed by media ID, model version and preprocessing version. Retried export/backup items are keyed by immutable job snapshot plus destination object identity. If remote outcome is unknown, query remote state before re-uploading or advancing job state.

## Progress semantics

Progress reports completed/total items and current stage. When total is unknown due to pagination, use indeterminate progress rather than invented percentages. A job is successful only when all required stages reach their defined completion condition; pending embeddings are surfaced separately rather than hidden inside a generic success count.


## One-shot job lifecycle

Ingestion jobs are finite and user-initiated. The user creates a job from source/search inputs; the job runs until it completes, is cancelled, fails into a persisted resumable state, or requires review. It must not restart or schedule itself after reaching a terminal state. Scheduled/continuous monitoring is not part of the initial ingestion workflow.

Each job is a separately persisted, resumable work unit with a stable ID, immutable input snapshot, cursor/checkpoint, stage, progress and per-item outcomes. A failed/cancelled job must not invalidate items already committed successfully. Keep this contract independent of source adapter implementation so providers can be implemented and verified one at a time.


## Independent DINOv3 analysis work

Successful import is defined by validated media and provenance committed to managed storage/database; it does not wait for embedding or visual similarity analysis. Enqueue DINOv3 analysis by default after import as a separate persisted work unit. If the model/runtime is missing or analysis fails, retain the imported item and report analysis as pending, retryable, failed or requiring review independently of import status.

Expose a separate batch-analysis/reanalysis action for existing library items. Key analysis results by media ID, model version and preprocessing version. Re-analysis creates versioned results and must not silently overwrite human-confirmed grouping or representative selections. Show import-job and analysis-job progress separately; users can still browse, review metadata and export an already imported item while its DINOv3 analysis is pending, subject to export filters.
