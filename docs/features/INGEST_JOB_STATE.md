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
