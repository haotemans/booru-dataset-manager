# Product decisions

This file records user-confirmed product decisions. A decision is not a claim that its feature is implemented.

## D-001 — Product scope

**Decision:** Build a complete integrated desktop tool covering source ingestion, media and identity management, metadata optimization, review, dataset export, dashboard statistics, and backup/restore.

**Implications:** Deliver in staged milestones, but keep a shared media library and shared application services. Avoid separate incompatible workflows for artist pages, character pages and global export. Preserve modular boundaries so source adapters, AI providers, model runtimes and backup transports can be added independently.

**Status:** Confirmed by user; architecture/documentation only.

## D-002 — Visual similarity policy

**Decision:** DINOv3 may automatically group visual-similarity candidates using a validated, versioned policy, but it must not delete original files. Users must be able to inspect and manually choose preferred images. The product should also support automatic selection of a recommended representative image where the configured policy and evaluation evidence justify it.

**Implications:** Distinguish visual grouping, representative selection and physical deletion as separate operations. Group membership does not imply byte-identical duplicates. A representative is a reversible dataset-selection decision, not a destructive cleanup. Exact byte duplicates may be deduplicated logically using database-enforced hashes while retaining source provenance.

**Status:** Confirmed by user; policy documentation only. Model thresholds and quality-ranking behavior are not yet calibrated or implemented.

## D-003 — Target library scale

**Decision:** Design for 100,000 to 1,000,000 media items.

**Implications:** Use SQLite indexes and batched queries; avoid loading the full library or all embeddings into UI memory. Separate embedding generation from interactive queries. Use paginated candidate retrieval, bounded queues, resumable jobs, cancellation, and measurable latency/memory budgets. Benchmark before selecting an approximate-nearest-neighbor index or adding a separate search service.

**Status:** Confirmed by user; performance has not been benchmarked.

## D-004 — Representative-image ranking and original retention

**Decision:** Rank representative candidates primarily by visual appeal and degree of completion, then by minimal visual distractions; images with no visible text are preferred. Keep all original files. Use a vision API to assess aesthetic quality and suggest potentially beneficial crops, but require human review before accepting representative choices or crop suggestions.

**Implications:**
- Treat aesthetic/completion quality as the primary preference, not file size, source priority, or embedding similarity.
- Prefer clean compositions with fewer distracting overlays or artifacts; text-free images are preferred when other quality factors are comparable.
- A vision API may propose crop coordinates and explain why the crop could improve composition. Example: for a 9:16 image, shifting the crop window upward may improve framing. The system must not assume every image needs cropping or that a crop improves aesthetics.
- Any crop is a derived preview/export variant, never an edit to the original. Preserve the original bytes and hash; show original and crop side by side, allow adjustment/rejection, and only use an approved crop in an explicitly selected output.
- Show recommendation reasons and uncertainty. Human decisions take precedence and are recorded in history.
- This is a desired workflow, not a claim that the vision API, crop UI, or ranking system is implemented or already reliable.

**Status:** User-confirmed preference; ranking, crop suggestions, human review UI and evaluation remain pending.

## D-005 — Vision API image-upload consent

**Decision:** Before a visual-analysis job sends images to an external vision API, the user must confirm upload for that batch. One confirmation may authorize the selected batch; no confirmation is assumed for future batches.

**Implications:**
- The confirmation screen must identify the provider/profile, selected image count and scope, the purpose of analysis, and any applicable cost/usage estimate available to the app.
- Clearly state that selected image content will leave the local machine and be sent to the configured external provider. Do not imply that only metadata is sent when image bytes are included.
- Provide reviewable selection and a way to remove images from the batch before approval. Cancel means no image upload for that job.
- Approval is scoped to the confirmed batch/job and purpose; it is not persistent global consent. A later batch requires a new confirmation, even if the same profile is used.
- Do not upload automatically in background jobs before confirmation. If confirmation cannot be obtained, skip external vision analysis and report that it requires approval.
- Never send API keys, cookies, provider session tokens, or unrelated local files with image requests. Redact credentials from diagnostics and logs.
- Store a minimal audit record of consent scope/time and job outcome without copying image bytes or secrets into the audit log. Do not treat consent as proof that a provider has a particular retention or training policy; display provider policy information separately when known.

**Status:** User-confirmed policy; UI, request gating, audit behavior and tests remain to be implemented.

## D-006 — Structured JSON AI requests and responses

**Decision:** AI integrations must send requests using JSON and parse responses as structured JSON for reliable batch processing. Do not depend on free-form prose or ad-hoc text extraction for successful batch results.

**Implications:**
- Define versioned schemas per operation, with explicit fields, types, enums, bounds and required identifiers.
- For batch work, include a stable per-item ID and require a corresponding result/status for each input item. Validate that IDs are unique, expected and not missing; do not associate results by array position alone.
- Prefer provider-supported structured output / JSON Schema or strict JSON mode when available. For providers without that capability, still request JSON and parse strictly.
- Validate syntax, schema, required fields, numeric ranges, crop coordinates and image/item identity before accepting any result. Reject malformed or ambiguous results and mark affected items for bounded retry or human review.
- Handle partial batch failure per item: preserve valid results, retry only eligible failed items with limits, and never let one malformed item corrupt or discard unrelated successful results.
- Bound request/response sizes and batch concurrency; persist job and item states so batches can resume without duplicating accepted work.
- Keep diagnostics redacted; retain provider/model/schema versions and validation outcomes for auditability.
- JSON validity is not semantic correctness. Protected-term checks remain required, and vision-based aesthetic/crop proposals remain advisory pending human review.

**Status:** User-confirmed integration requirement; schemas, parser, retry behavior and tests remain to be implemented.

## D-007 — AI batch size and failure-window advancement

**Decision:** The default AI request batch contains 8 items. A fully successful 8-item window advances by 8. If the request fails as a whole, advance by 4 to create a 4-item overlap. If a response is valid but only some items fail or are missing, preserve successful item results and recover only the failed item group; the normal 8-item main window continues advancing by 8. At a tail that cannot advance at the current window size, reduce batch size progressively: 8 → 4 → 2 → 1, advancing by the active fallback batch size.

**Implications:**
- Distinguish transient transport failure, batch-processing failure, and item-level partial failure. Network timeouts and connection interruptions are transient transport failures: retry the same batch with backoff and do not shrink the batch because of these errors. Malformed JSON or response-level schema failure are batch-processing failures; item-level failure means a valid response identifies failed items or omits expected item results.
- If bounded retries for a transient network failure are exhausted, defer the affected item IDs and continue the main scan rather than shrinking the batch or blocking progress. Preserve their order and failure provenance, then retry them during post-scan recovery when connectivity is available. If connectivity remains unavailable, leave them pending for a later resume instead of classifying them as content failures.
- For a partial response, accept and persist every valid item result. Build an independent recovery group from only the failed/missing item IDs; do not reprocess successful items as part of that recovery group.
- Queue failed item groups while the main-window scan continues. Do not run recovery requests during the main scan; drain the recovery queue only after the main scan reaches its end. This keeps recovery work from interrupting normal forward progress.
- Deduplicate the recovery queue by stable item ID, not by window or group. A given item has at most one pending recovery entry; merge duplicate failure references into that entry rather than scheduling duplicate API work. Before dispatch, re-check the item's current state and skip it if a valid result is already accepted or its content has been human-reviewed. Keep audit history separately from the deduplicated work queue.
- After the main scan ends, form recovery requests by taking the deduplicated pending item IDs in queue order and regrouping them into batches of up to 8 items. A final recovery batch may contain fewer than 8 items; do not pad it. Preserve stable IDs and audit references when regrouping.
- Apply the same item-level partial-success handling to recovery batches: keep valid results, and enqueue only failed items for a later recovery batch. If a recovery request fails as a whole, use the documented fallback policy to split only that failed batch progressively (8 → 4 → 2 → 1); do not resend an identical failed payload indefinitely.
- Recovery is a separate fallback phase, not a same-payload, same-size retry. Process queued groups in queue order. If a recovery group still fails, subdivide only that failed group progressively (for example, 2 failed items become two single-item requests); never discard previously accepted results.
- Batch-processing failures in the main scan use the 4-item cursor advance and overlap policy. Transient network timeouts or connection interruptions do not advance by 4 and do not create overlap solely due to transport failure; retry the same batch with backoff. Track unresolved item IDs so no item is silently lost.
- A failed whole 8-item window advances the main cursor by 4, creating a 4-item overlap. A successful 8-item window—or a valid partial response whose failed items are handled by the separate recovery phase—advances the normal main cursor by 8.
- Track every result by stable item ID. Writes must be idempotent; overlap and recovery requests must not duplicate accepted results or overwrite human-reviewed choices.
- Do not repeat an identical batch-processing failure indefinitely. At a tail, reduce batch size progressively: 8 → 4 → 2 → 1. Transient network failures use bounded same-batch retries with backoff, not batch shrinking. Within each fallback level, advance by that level's batch size. A failed 4-item fallback group is subdivided into 2-item groups; a failed 2-item group is subdivided into single-item groups.
- A failed single-item request is recorded as failed/deferred for later manual handling; do not retry indefinitely.
- Persist the main cursor, recovery groups, fallback level and per-item states for resume. The final window may contain fewer items than the current batch size; never add fake padding entries.
- Mark the job complete only when every input item has a terminal outcome: successfully completed, or explicitly recorded as a final failed/deferred item after content-processing fallbacks. Items deferred solely because of transient network failure are non-terminal; keep the job resumable and not complete while any such item remains pending. Resume post-scan recovery when connectivity returns.

- On application startup, detect persisted incomplete AI jobs and show the user a resume prompt with the saved job status/progress. Do not issue external API requests until the user explicitly chooses to resume. Dismissal leaves the job pending; the user can resume it later from the task UI.

- On resume, reconcile any request that was in flight at interruption before blindly resending it. Prefer provider-supported request-status lookup and/or idempotency keys using a stable persisted request ID. If the provider offers neither capability, apply a documented bounded fallback that avoids indefinite retries and records ambiguity; do not assume exactly-once execution.

**Status:** User-confirmed processing policy; scheduler, persistence and tests remain to be implemented.
## D-008 — AI request timeout and error-specific retry policy

**Decision:** Set the default AI request timeout to 120 seconds and allow at most one retry per request. Retry behavior must depend on error category rather than applying the same retry rule to every failure.

**Implications:** Retry transient connection failures/timeouts at most once using a bounded backoff. For HTTP 429, honor `Retry-After` when present, while retaining the one-retry limit. Retry eligible HTTP 5xx failures at most once. Do not automatically retry authentication/authorization errors or other clearly non-transient client errors. A timeout or connection loss remains a transport failure: do not shrink the batch or apply the content-processing overlap rule solely because of it. If the single retry fails, persist unresolved item IDs for later recovery; do not loop indefinitely. Resume reconciliation and provider idempotency/status support still apply.

**Status:** User-confirmed policy; implementation and tests remain to be completed.

## D-009 — AI API concurrency

**Decision:** The default maximum number of concurrent AI API requests is 2. The concurrency limit is configurable per provider profile.

**Implications:** Enforce the configured limit centrally in the request scheduler; batch size 8 does not imply eight simultaneous requests. Validate configured values and apply a bounded safe range. Respect provider rate limits and HTTP 429 retry policy without silently increasing concurrency. Persist enough job state to resume safely after interruption.

**Status:** User-confirmed policy; implementation and tests remain to be completed.

## D-010 — Adaptive concurrency on HTTP 429

**Decision:** Dynamically reduce AI API concurrency when HTTP 429 rate limiting is observed, then cautiously restore it after a stable period.

**Implications:** Honor `Retry-After` when supplied and keep the existing maximum of one retry per request. Reduce the active concurrency limit conservatively (never below 1); do not increase above the user-configured profile limit. Restore concurrency gradually only after a defined cooldown without further 429 responses. A 429 must not trigger batch shrinking or content-processing fallback. Persist unresolved item IDs and avoid retry loops.

**Status:** User-confirmed policy; exact reduction/recovery parameters and implementation remain to be completed.

## D-011 — Concurrency scope per provider profile

**Decision:** All jobs using the same AI API provider profile share one concurrency limit and one adaptive rate-limit controller. Different profiles maintain independent limits.

**Implications:** Enforce concurrency in a shared scheduler keyed by provider profile, not separately per task. Tag optimization and caption generation using the same profile count toward the same cap. HTTP 429 adaptation applies to that profile's shared active limit and must never exceed its configured maximum. Jobs using other profiles are not throttled by this profile's 429 response.

**Status:** User-confirmed policy; implementation and tests remain to be completed.

## D-012 — Runtime concurrency configuration changes

**Decision:** User changes to a provider profile's configured concurrency limit take effect immediately for future scheduling, but cannot override a lower active limit imposed by HTTP 429 rate-limit adaptation.

**Implications:** Keep the configured maximum and dynamic active limit as separate values. Effective concurrency is bounded by both; increasing the configured value does not clear an active rate-limit reduction. Gradual recovery may raise the active limit only under the established cooldown rule and never above the latest configured maximum. Reducing the configured value should stop dispatching new work until in-flight requests fall within the new cap; do not cancel already-sent requests solely due to this setting change.

**Status:** User-confirmed policy; implementation and tests remain to be completed.

## D-013 — Concurrency reduction after HTTP 429

**Decision:** When HTTP 429 is received, reduce the active concurrency limit to half its current value, rounded up, with a minimum of 1.

**Implications:** Example: `8 → 4 → 2 → 1`. Apply the reduction to the affected provider profile's shared scheduler. The active limit may never exceed the latest user-configured maximum. Honor `Retry-After` when present and retain the existing maximum of one retry per request. Restore concurrency gradually only after the configured cooldown without further 429 responses; exact recovery increment/cooldown parameters remain to be specified.

**Status:** User-confirmed policy; implementation and tests remain to be completed.

## Deferred decisions

- Which vision API/provider to use, its privacy/cost controls, and provider-specific retention/training policy.
- Exact ranking weights and how to evaluate aesthetic completion, clutter, and crop quality against human judgments.
- Whether automatic representative recommendations are enabled by default for newly discovered groups.
- Whether a visual group can contain different crops/poses/frames versus only near-duplicate versions. The relation types must remain distinct even if groups share one UI.
- Which DINOv3 artifact/runtime and nearest-neighbor index meet license, compatibility, memory and latency requirements.
