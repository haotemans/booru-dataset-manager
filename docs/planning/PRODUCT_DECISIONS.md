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
- Bound request/response sizes and batch concurrency; persist job/item state so batches can resume without duplicating already accepted work.
- Keep diagnostics redacted; retain provider/model/schema versions and validation outcomes for auditability.
- JSON validity is not semantic correctness. Protected-term checks remain required, and vision-based aesthetic/crop proposals remain advisory pending human review.

**Status:** User-confirmed integration requirement; schemas, parser, retry behavior and tests remain to be implemented.

## D-007 — AI batch size and failure-window advancement

**Decision:** The default AI request batch contains 8 items. Single-item processing must also be supported. When a batch fails, advance the input window by 4 items and continue, creating a 4-item overlap with the prior window.

**Implications:**
- For a successful batch, advance by 8 items. For a failed batch, advance by 4 items; do not reprocess already accepted item results just because a later window overlaps them.
- Track state and outcome per item ID. Deduplicate accepted results by job and item ID so overlap cannot create duplicate writes or overwrite human-reviewed results.
- Preserve valid per-item results from a partially successful response. Treat invalid/missing/failed item results as failures, rather than discarding unrelated valid results.
- Support a batch size of 1 for providers/models or tasks that are more reliable with single-item requests. Batch size should be configurable, with 8 as the default.
- Bound retries per item and per request. A failed window must not cause an infinite loop; persist the failure reason and leave exhausted items in a retry/review queue.
- Handle the final window when fewer than 8 items remain without padding with fake items. Confirm every input item has a terminal or explicitly deferred state before marking the job complete.

**Status:** User-confirmed processing policy; scheduler, persistence and tests remain to be implemented.

## Deferred decisions

- Which vision API/provider to use, its privacy/cost controls, and provider-specific retention/training policy.
- Exact ranking weights and how to evaluate aesthetic completion, clutter, and crop quality against human judgments.
- Whether automatic representative recommendations are enabled by default for newly discovered groups.
- Whether a visual group can contain different crops/poses/frames versus only near-duplicate versions. The relation types must remain distinct even if groups share one UI.
- Which DINOv3 artifact/runtime and nearest-neighbor index meet license, compatibility, memory and latency requirements.
