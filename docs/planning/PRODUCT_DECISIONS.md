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

**Decision:** For source-confirmed difference sets, default to automatically selecting the single most aesthetically appealing image as representative; the user may manually replace that choice at any time. Evaluate aesthetic appeal and degree of completion first, then consider visual distractions and visible text as supporting factors. Keep all original files. Crop suggestions still require explicit human approval.

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

**Decision:** When HTTP 429 is received, disable concurrent dispatch for the affected provider profile by setting its active concurrency limit to 1.

**Implications:** After a 429, process requests serially for that profile; do not send parallel requests while this degraded mode is active. This overrides the previous halve-current-limit rule. Honor `Retry-After` when present and retain the existing maximum of one retry per request. For recovery, honor a server-provided `Retry-After` as the cooldown when present; if absent, use a default cooldown of 5 minutes. Once the cooldown elapses without another 429, restore concurrency gradually by increasing the active limit by 1 at a time, never above the latest user-configured maximum. Any new 429 resets the cooldown and returns the active limit to 1.

**Status:** User-confirmed policy; implementation and tests remain to be completed.

## Deferred decisions

- Which vision API/provider to use, its privacy/cost controls, and provider-specific retention/training policy.
- Exact ranking weights and how to evaluate aesthetic completion, clutter, and crop quality against human judgments.
- Whether automatic representative recommendations are enabled by default for newly discovered groups.
- Whether a visual group can contain different crops/poses/frames versus only near-duplicate versions. The relation types must remain distinct even if groups share one UI.
- Which DINOv3 artifact/runtime and nearest-neighbor index meet license, compatibility, memory and latency requirements.


## D-014 — Serial request pacing after HTTP 429

**Decision:** After a provider profile enters serial mode following HTTP 429, apply a configurable minimum interval between request starts, defaulting to 1 second. This interval remains in force after the cooldown expires and while requests are serial; it is separate from the 429 cooldown and any server-provided `Retry-After` delay.

**Implications:** Requests for the affected profile must not overlap while active concurrency is 1, and the scheduler must enforce the configured spacing between request starts. The 429 cooldown follows D-013: use `Retry-After` when present, otherwise 5 minutes; a new 429 resets the active concurrency to 1 and restarts the cooldown. Keep pacing configuration scoped to the provider profile.

**Status:** User-confirmed policy; implementation and tests remain to be completed.


## D-015 — Serial request interval timing

**Decision:** The minimum request interval in serial mode is configurable, defaults to 1 second, and is measured from the start of the previous request to the start of the next request. If the previous request itself takes at least the configured interval, do not add extra waiting time after it completes.

**Implications:** The scheduler enforces request-start spacing rather than a fixed post-completion sleep. This interval is separate from the HTTP 429 cooldown and applies while the profile is in serial mode. A new 429 still resets the active concurrency limit to 1 and restarts the cooldown.

**Status:** User-confirmed policy; implementation and tests remain to be completed.


## D-016 — Transient network failure during serial dispatch

**Decision:** In serial mode, transient network failures and timeouts follow the existing transport retry policy: retry the same request at most once with bounded backoff. If the retry also fails, persist the affected item IDs as pending network recovery and continue processing subsequent items; do not shrink the batch or classify the items as content-processing failures solely because transport failed.

**Implications:** Pending items are recovered after the main scan when connectivity is available, remain resumable if connectivity is still unavailable, and do not count as completed outcomes. Apply the same request-start pacing rule to subsequent serial requests. HTTP 429 remains governed by the dedicated cooldown and concurrency rules.

**Status:** User-confirmed policy; implementation and tests remain to be completed.


## D-017 — Handling HTTP 429 on the current serial request

**Decision:** When a request receives HTTP 429, honor `Retry-After` when present; otherwise use the default 5-minute cooldown. After the cooldown, retry the same request once, subject to the existing maximum of one retry per request. If that retry still fails, persist the affected item IDs for later recovery and continue processing subsequent items.

**Implications:** A 429 immediately sets the affected provider profile's active concurrency to 1 and resets its cooldown. Do not shrink the batch or classify the item as a content-processing failure solely due to rate limiting. Keep the configured serial request-start interval in force. A later 429 starts a new cooldown; unresolved IDs remain resumable.

**Status:** User-confirmed policy; implementation and tests remain to be completed.


## D-018 — Provider-profile-wide rate-limit gate

**Decision:** All tasks using the exact same configured API profile share one rate-limit scheduler and cooldown gate. Distinct API configurations do not share state merely because they use the same provider name; they share only if they use the same profile entity or are explicitly assigned to the same rate-limit group. When any task receives HTTP 429, its applicable shared scheduler blocks new request dispatch for all tasks governed by that scheduler for the cooldown. After cooldown, dispatch resumes in serial mode, then concurrency is gradually restored under the applicable profile/group rules.

**Implications:** A task cannot bypass a cooldown by issuing requests through another worker that uses the same profile. Other provider profiles remain independent. Persist profile-level cooldown and active concurrency state so task pause/resume or application restart does not inadvertently bypass the gate.

**Status:** User-confirmed policy; implementation and tests remain to be completed.


## D-019 — Persisted cooldown deadline across restarts

**Decision:** Persist an absolute cooldown deadline for each provider profile. On application restart, wait only until that deadline; if it has already passed, the cooldown is considered elapsed and the scheduler may resume according to the existing serial-first recovery policy.

**Implications:** Persist the deadline in durable storage with the profile-level rate-limit state. Account for system clock changes defensively: if the wall clock is moved backward or the deadline cannot be trusted, fail conservatively by retaining serial mode and re-establishing a safe cooldown rather than raising concurrency unexpectedly. Do not reset the cooldown to a full duration on every restart.

**Status:** User-confirmed policy; implementation and tests remain to be completed.


## D-020 — Gradual concurrency recovery after HTTP 429

**Decision:** After the applicable profile/group cooldown expires, remain at active concurrency 1 until a configurable number of consecutive fully successful batches has completed; default 5. Then increase the active concurrency limit by 1. For each subsequent increase, require another 5 fully successful batches at the current active limit by default. Both success-count thresholds are configurable; never exceed the latest applicable configured maximum.

**Implications:** Persist the recovery progress per profile or shared rate-limit group so restarts do not reset it accidentally. A new HTTP 429 immediately resets the applicable active concurrency to 1, restarts the cooldown, and resets recovery progress. Only fully successful batches completed after cooldown expires count; failures, partial batches, cooldown-period completions, and pending network recovery do not count as successes.

**Status:** User-confirmed policy; implementation and tests remain to be completed.


## D-021 — Success criterion for concurrency recovery

**Decision:** A request counts toward gradual concurrency recovery only when the HTTP request succeeds, the response passes format/schema validation, and every item in the active batch is processed successfully. Partial responses, malformed responses, or batches with any failed item do not count as successful recovery requests.

**Implications:** Preserve valid partial results according to the existing partial-response policy, but do not increment the concurrency recovery success counter for that batch. Only fully successful batches advance the counter. A new HTTP 429 resets the counter and cooldown as specified in D-020; other failed batches do not count as successes.

**Status:** User-confirmed policy; implementation and tests remain to be completed.


## D-022 — Which failures reset concurrency recovery progress

**Decision:** Only a new HTTP 429 resets the concurrency recovery success counter and restarts the cooldown. Other failed requests or batches do not increment the counter but preserve the already accumulated successful-batch progress.

**Implications:** Keep transport/content-processing failure handling separate from rate-limit recovery. Valid partial results may be preserved under the existing policy, but the batch does not count as successful. Do not clear the recovery counter for ordinary errors; a new 429 always returns active concurrency to 1 and resets the counter.

**Status:** User-confirmed policy; implementation and tests remain to be completed.


## D-023 — Ordinary failures do not reduce concurrency

**Decision:** After concurrency has recovered above 1, ordinary failures such as HTTP 500 or content-processing failures do not reduce the active concurrency limit by themselves. Handle them using the existing error-specific retry and recovery policies. HTTP 429 remains the explicit signal that resets concurrency to 1 and restarts the cooldown.

**Implications:** Do not infer rate limiting from an arbitrary failed batch. Only fully successful batches increment recovery progress; ordinary failures preserve prior progress without incrementing it, as specified in D-022. Eligible HTTP 5xx requests may still receive the existing single bounded retry, but do not change concurrency solely because of the 5xx.

**Status:** User-confirmed policy; implementation and tests remain to be completed.


## D-024 — Raising configured concurrency during rate-limit degradation

**Decision:** Changing the user-configured maximum concurrency does not override an active rate-limit reduction. If the configured maximum is raised while the active limit is 1 after HTTP 429, keep the active limit at 1 and recover gradually under the existing cooldown and successful-batch thresholds, up to the new configured maximum.

**Implications:** The effective concurrency limit must always be no greater than both the configured maximum and the current rate-limit-adapted active limit. A configured maximum change affects the ceiling for future recovery, not the cooldown, recovery counter, or current active limit. Lowering the configured maximum takes effect for future dispatches without cancelling already in-flight requests solely due to the setting change.

**Status:** User-confirmed policy; implementation and tests remain to be completed.


## D-025 — Configurable shared rate-limit groups

**Decision:** Each AI API configuration is independent by default. Users may explicitly place multiple configurations into the same rate-limit group; configurations in a shared group use one shared concurrency cap, active adaptive limit, cooldown, and recovery counter.

**Implications:** Do not infer a shared limit merely because configurations use the same provider name. Group membership is explicit and persisted. Requests dispatched through any member must respect the group's effective concurrency and cooldown gate. Configurations outside the group remain independent. If a configuration is moved between groups while requests are in flight, do not cancel those requests solely because of the change; apply the new group's gate to future dispatches and reconcile in-flight accounting safely.

**Status:** User-confirmed policy; configuration UI, persistence, scheduler integration, and tests remain to be completed.


## D-026 — Member and group concurrency caps

**Decision:** Each API configuration retains its own configured maximum concurrency. A shared rate-limit group additionally has a separately configured group-wide maximum. Dispatch must respect both the member's effective limit and the shared group's effective limit; the total in-flight requests across group members must never exceed the group's effective cap.

**Implications:** Member limits constrain requests from that configuration, while the group cap constrains aggregate requests across all members. The group cap and adaptive active limit are separate from each member's configured maximum. Existing cooldown and gradual recovery behavior applies at group scope when configurations share a rate-limit group. Configurations outside a group remain independently governed. Changes to limits affect future dispatches and do not cancel in-flight requests solely because settings changed.

**Status:** User-confirmed policy; settings UI, persistence, scheduler enforcement, and tests remain to be completed.


## D-027 — HTTP 429 cooldown scope for shared rate-limit groups

**Decision:** When any API configuration in a shared rate-limit group receives HTTP 429, the entire group enters cooldown. Stop dispatching new requests for all configurations in that group. When cooldown expires, resume from group concurrency 1 and use the existing gradual recovery policy. The request that received 429 follows the existing policy: honor Retry-After when present, otherwise use the configured default cooldown; retry that request at most once, then defer affected item IDs to recovery if it fails again.

**Implications:** Group cooldown and adaptive concurrency state are shared across all group members. Other in-flight requests are not cancelled solely because a member triggered 429; they may complete, but no new group requests dispatch until cooldown expires. Configurations outside the group are unaffected. The 429 resets the shared group's recovery progress.

**Status:** User-confirmed policy; scheduler integration, persistence, and tests remain to be completed.


## D-028 — Default maximum concurrency for a new shared rate-limit group

**Decision:** A newly created shared rate-limit group defaults to a group-wide maximum concurrency of 2, regardless of the sum or highest value of member configurations' individual maximums. Users may explicitly increase or decrease the group maximum.

**Implications:** Creating a shared group must not silently increase aggregate request pressure. Each member remains constrained by its own configured/effective limit as well as the group's aggregate/effective limit. The group's default maximum is independent of member count and member-level settings. Existing HTTP 429 cooldown and gradual recovery rules remain in force.

**Status:** User-confirmed policy; settings UI, persistence, scheduler integration, and tests remain to be completed.


## D-029 — Requests completing during a shared-group cooldown

**Decision:** Successful requests that were already in flight when a shared rate-limit group entered HTTP 429 cooldown do not count toward gradual concurrency recovery, even if they complete successfully during the cooldown. After the cooldown expires, recovery success counting starts at zero.

**Implications:** A shared-group 429 resets the group's recovery counter. Cooldown-period completions may still persist valid item results, but they do not advance the adaptive concurrency recovery counter. Once cooldown ends, only fully successful batches completed under the post-cooldown recovery phase count toward the configured threshold for raising concurrency.

**Status:** User-confirmed policy; scheduler implementation and tests remain to be completed.


## D-030 — Saving unverified AI API configurations

**Decision:** Users may save an AI API configuration without first completing a successful connection test. Connection testing is available as an explicit action, and task execution reports connection, authentication, or model errors if the saved configuration is not usable.

**Implications:** Keep configuration persistence separate from connection validation state. The UI must distinguish saved/unverified, test successful, and test failed or stale states rather than implying a saved configuration is known to work. Saving a configuration must not itself trigger external API requests. Credentials must be stored using the application's credential-protection approach and must never appear in logs or diagnostic output.

**Status:** User-confirmed policy; settings UI, validation-state persistence, and tests remain to be completed.


## D-031 — Pause jobs on configuration-wide AI API errors

**Decision:** When an AI request returns a clear configuration-wide error—such as HTTP 401 authentication failure, HTTP 403 authorization failure, or an explicit unsupported/nonexistent model error—pause the affected job immediately. Preserve the cursor, recovery queue, item outcomes, and all resumable progress. Show an actionable error so the user can correct the saved API configuration, then require explicit user action to resume.

**Implications:** Do not repeatedly send requests known to fail because of the same configuration issue, and do not classify affected items as content-processing failures. Ordinary item-specific failures continue through existing bounded retry and recovery policies. A failure isolated to one API profile pauses jobs using that profile; unrelated jobs on independent profiles may continue. If the error occurs within a shared rate-limit group, pause the affected job and do not bypass any active group cooldown. Configuration edits alone do not automatically resume or dispatch requests.

**Status:** User-confirmed policy; error classification, job pause/resume integration, UI, persistence, and tests remain to be completed.


## D-032 — One-shot ingestion jobs and independently deliverable features

**Decision:** The first ingestion workflow is user-initiated and finite: create a one-shot job from source/search inputs, run it to a terminal or resumable state, and stop. Scheduled or continuous source monitoring is out of scope for the initial ingestion workflow. Features should be implemented as independently testable modules with explicit contracts and dependencies, and may be delivered in stages rather than requiring the entire product to be completed at once.

**Implications:** Persist each job's input snapshot, source cursor, progress, item outcomes, cancellation state and resumable state. One job must not silently turn into a recurring task. Implement and test the ingestion workflow independently behind application/domain/port boundaries; other features can be added later without duplicating the ingestion pipeline or bypassing shared media and job services.

**Status:** User-confirmed policy; implementation and tests remain to be completed.


## D-033 — Danbooru primary source with Gelbooru fallback

**Decision:** Danbooru is the preferred/default source for ingestion. Gelbooru is the fallback source when the current task cannot feasibly proceed through Danbooru. This is a prioritized failover policy, not default parallel harvesting from both sources.

**Implications:** Keep source selection and fallback decisions visible in the job's provenance and progress. Records from either provider still flow through the same normalization, dimension gate, exact-deduplication and media-import pipeline. A fallback must not bypass source authentication, provider policy, rate limits, or access restrictions. The exact conditions that trigger fallback must be explicitly defined before implementing automatic failover.

**Status:** User-confirmed provider priority; automatic fallback conditions, implementation and tests remain to be specified.


## D-034 — Conditions for falling back from Danbooru to Gelbooru

**Decision:** In a Danbooru-first ingestion task, attempt Gelbooru when Danbooru cannot be used for the task or yields no items that pass the task's configured eligibility filters (including the minimum resolution and validated-file requirements). A successful search with no results, or a result set whose items are all excluded by eligibility filters, may trigger fallback. Reaching a numeric target before exhausting Danbooru results does not itself require Gelbooru; top-up behavior is not part of this fallback rule.

**Implications:** Continue to respect Danbooru authentication, rate limits, Retry-After, cooldowns and provider policies. A transient failure or 429 must follow the provider's retry/cooldown policy before any failover decision; do not use Gelbooru to bypass throttling or access restrictions. Record the fallback reason and provider provenance. De-duplicate items across providers through the shared media pipeline.

**Status:** User-confirmed fallback policy; exact error classification, implementation and tests remain to be completed.


## D-035 — Tiered automation for visual grouping

**Decision:** Use tiered handling. After a versioned policy has been validated against labeled evaluation data, high-confidence near-duplicate variants such as resized or recompressed versions may be grouped automatically. Images that differ by pose, scene, crop/content, character identity, or are merely similar in composition/style are candidate relations for human review rather than automatic identity grouping based on similarity alone.

**Implications:** Exact byte duplicates remain a separate hash-based concept. Visual grouping never deletes or overwrites original media. Automated group decisions must be reversible and record the model, preprocessing and policy versions; human decisions take precedence and retain audit history. If the evaluated policy does not support safe automatic grouping, leave the items as candidates for review instead of inventing a threshold.

**Status:** User-confirmed policy; relation classification, labeled evaluation, calibrated policy, implementation and tests remain to be completed.


## D-036 — Broad visual-association groups and difference sets

**Decision:** A visual-association group may contain more than near-duplicate images, including difference sets such as related artwork variants from the same work. Every member relationship must have an explicit relation type; membership does not imply exact duplication or interchangeability. Automatically establish an association when the source explicitly provides a trustworthy difference-set relationship, recording its provenance. Other broad associations may be human-confirmed when needed. DINOv3 similarity alone must not automatically assert that different poses, scenes, character identities or merely similar compositions belong to the same association group.

**Implications:** Keep relation classes distinct, including exact duplicate, resized/recompressed near-duplicate, crop/content variant, difference/alternate artwork, alternate pose/frame, same subject across scenes, same character/person, same artist, and merely similar composition/style. Representative-image selection is restricted to a subset whose members are reasonably interchangeable for the relevant export/review purpose; do not select one representative for an entire broad association group by default. Preserve original media, source provenance, relationship source and human override history. Source relationships must not bypass exact-hash deduplication rules or imply file deletion.

**Status:** User-confirmed policy; source relationship ingestion, group/member data model, UI, representative scope and tests remain to be implemented.


## D-037 — Automatic association of source-confirmed difference sets and single representative selection

**Decision:** When a provider explicitly and reliably identifies a difference set (for example, a Danbooru difference relationship), automatically create the association group and record the source relationship. For representative-only dataset selection, automatically choose the most aesthetically appealing single representative per difference set by default; the user may manually choose a different member at any time.

**Implications:** Automatic selection follows the existing preference order: aesthetic appeal/completion, fewer visual distractions, and preferably no visible text when other factors are comparable. Store the selected representative, selection mode, ranking/policy version, recommendation rationale and decision history. Manual choice overrides automatic selection. Keep all original media files and provenance in the library; “one representative” governs the representative-only dataset/export view and does not physically delete or erase unselected difference variants. Exports may offer an explicit all-variants option. Do not apply one representative to an entire broad association group unless its members are confirmed to be interchangeable for that selection purpose. Crop suggestions remain subject to explicit human approval.

**Status:** User-confirmed policy; source relationship extraction, representative selection modes, export behavior, persistence, implementation and tests remain to be completed.


## D-038 — Decouple ingestion from DINOv3 analysis

**Decision:** After a media item has passed download validation, dimension checks, exact-deduplication and the media/metadata commit, enqueue DINOv3 embedding and visual-relation analysis by default as a separate background work unit. DINOv3 must not be a prerequisite for successful media ingestion. Users also have a separate batch-processing/reanalysis entry point for existing library items.

**Implications:** If the model/runtime is unavailable, keep the item imported and mark analysis as pending with an actionable reason. If embedding or similarity analysis fails, retain the imported media, preserve any valid prior analysis and record a retryable/permanent analysis state independently. Re-analysis is versioned by model and preprocessing version and must not silently overwrite human-confirmed relationships or representative selections. Expose ingestion completion and DINOv3 analysis completion as separate progress/status values.

**Status:** User-confirmed policy; queue, persistence, reanalysis UI, integration and tests remain to be implemented.


## D-039 — Aesthetic-first automatic representative selection

**Decision:** Automatically choose one representative for each eligible source-confirmed difference set, with aesthetic quality as the primary objective. The user can manually replace the selected representative at any time.

**Implications:** Use vision-based aesthetic assessment and quality evidence to rank candidates. Prioritize overall visual appeal and degree of completion; use visual clutter, distracting artifacts and visible text as supporting factors rather than rigid rejection rules. Do not substitute resolution, source priority, file size or DINOv3 similarity for aesthetic quality. If the assessment service is unavailable or cannot provide a sufficiently reliable ranking, leave the group marked as needing representative selection instead of silently choosing arbitrarily. Preserve all original variants, record selection rationale and model/prompt/policy versions, and keep manual overrides with audit history. Crop proposals still require explicit human approval.

**Status:** User-confirmed policy; aesthetic-ranking implementation, evaluation, fallback state and tests remain to be completed.


## D-040 — Tiered, indexed visual candidate retrieval

**Decision:** Use a layered retrieval strategy for each newly imported image. First reconcile exact duplicates through the hash/database uniqueness path and inspect known source-confirmed difference-set relationships; then retrieve indexed high-similarity near-duplicate candidates; finally search the broader library for cross-source visual-association candidates. Provider/source boundaries must not prevent cross-source discovery.

**Implications:** Use persistent vector/index infrastructure and bounded top-k candidate queries rather than loading the full library's embeddings into memory for every image. The broad search may propose relationships but does not automatically establish identity or grouping except for trustworthy source-provided relations and separately validated near-duplicate automation. Keep exact duplicate, near duplicate, difference set, and semantic/visual association as distinct relation types. Index rebuilds and model/preprocessing upgrades must be resumable and versioned; if an index is unavailable or stale, report degraded analysis and schedule repair rather than silently claiming a complete search.

**Status:** User-confirmed retrieval strategy; index technology, candidate limits, query budgets, implementation and performance tests remain to be specified.


## D-041 — Confidence-tiered near-duplicate handling

**Decision:** Exact duplicates are identified by content hash and database uniqueness constraints. For non-exact variants, use confidence-tiered handling: high-confidence resize/recompression near-duplicates may be automatically grouped only under a versioned policy validated against labeled examples; medium- and low-confidence candidates are presented for human confirmation. Preserve all files and provenance, and allow all automated decisions to be undone.

**Implications:** Confidence must be calibrated on representative labeled data and include hard negatives such as crops that alter content, alternate artwork/difference images, different poses/scenes, and similar compositions. Do not hard-code an arbitrary similarity threshold. Human-confirmed decisions take precedence over automated grouping and must retain audit history. Near-duplicate grouping is distinct from source-confirmed difference-set association and must not collapse meaningful variants in export.

**Status:** User-confirmed policy; calibration, confidence bands, implementation and tests remain to be completed.


## D-042 — Versioned visual embeddings and background migration

**Decision:** Model, exported runtime/model artifact, preprocessing configuration and embedding schema are versioned together. Upgrades retain the currently active embedding/index while a new version is built in the background. Queries continue using the active compatible index until the new index is validated and atomically promoted. Interrupted or failed rebuilds are resumable; the old index remains available for rollback.

**Implications:** Never compare embeddings from incompatible versions as if they shared one space. Track per-image analysis status and version, index build status, errors and progress. Rebuild work is idempotent and bounded; newly ingested images must be scheduled for the target version while migration runs. Promotion requires coverage/integrity checks and retrieval evaluation. Retain prior version metadata and enough state to rebuild or roll back; garbage-collect old vectors only after successful promotion and an explicit retention policy.

**Status:** User-confirmed architecture; implementation and operational thresholds remain to be completed.


## D-043 — Group visual search results by relationship type

**Decision:** Present visual retrieval candidates grouped by relationship type, then sort candidates within each group by calibrated confidence/relevance. Similarity score is not itself a relationship label and must not imply that two images are duplicates or interchangeable.

**Implications:** Keep distinct groups for exact duplicates, resize/recompression near-duplicates, crop/content variations, difference/alternate artwork, alternate poses/frames, same subject in another scene, same artist, and similar composition/style where evidence supports those labels. Show the relationship type, confidence or ranking signal, and evidence/provenance where available. Only actions appropriate to the relationship type are enabled; broad visual similarity must not be treated as permission to collapse variants or select a representative image. Unknown/ambiguous relationships remain unclassified candidates for review.

**Status:** User-confirmed presentation policy; UI implementation and usability validation remain to be completed.


## D-044 — Persistent human overrides for visual relationships

**Decision:** Human-confirmed visual relationship decisions are persisted as explicit overrides and survive model upgrades, index rebuilds and re-analysis. Automated analysis must not silently replace them. A user may explicitly edit or clear an override to allow reassessment.

**Implications:** Store the decision, relationship type, involved media IDs, actor/source, timestamp and audit history. Distinguish an explicit human override from a model-generated suggestion. Re-analysis may refresh scores and candidates but must preserve the human-selected relationship state; if the underlying media or relationship schema changes incompatibly, flag the decision for review rather than silently reinterpret it. Clearing an override is an explicit user action and should be auditable.

**Status:** User-confirmed policy; persistence, migration and UI implementation remain to be completed.


## D-045 — Bounded retry for visual analysis failures

**Decision:** DINOv3 analysis failures are retried automatically up to three attempts by default. After the retry budget is exhausted, the item is marked failed/deferred with an actionable error and a manual retry option. Visual analysis failure never rolls back successful media ingestion or blocks unrelated gallery operations.

**Implications:** Retry only retryable failures; permanent configuration/model incompatibility should be surfaced promptly rather than pointlessly retried. Persist attempt count, error category, last error and next retry/terminal state so restarts do not reset the retry budget. Use bounded backoff for transient failures and keep analysis queue state separate from ingestion state. Manual retry explicitly starts a new retry cycle while retaining prior failure history.

**Status:** User-confirmed policy; failure taxonomy, backoff values and implementation remain to be completed.


## D-046 — Prioritize newly ingested media in visual analysis

**Decision:** Schedule DINOv3 analysis for newly ingested media ahead of bulk backfill/re-analysis of existing library items. Backfill runs in the background and uses remaining processing capacity, so a large historical backlog does not indefinitely delay analysis of new media.

**Implications:** Keep ingestion state separate from visual-analysis state. Queue priority must be persisted and scheduling fair enough to ensure backfill eventually progresses rather than starving indefinitely. Failed items retain their retry policy and should not block unrelated items. Leave room for future explicit user priority controls without requiring them for the initial implementation. Expose new-item and backfill queue progress separately.

**Status:** User-confirmed scheduling policy; fairness details and implementation remain to be completed.


## D-047 — Safe pause and cancellation for visual analysis jobs

**Decision:** Visual analysis jobs support pause and cancel. Cancellation stops queued work that has not started and asks in-flight work to stop at safe cancellation points. Keep already committed valid analysis results, embeddings and index data; never remove media files or human-confirmed relationship overrides as a side effect of cancellation. Ingestion and unrelated jobs continue independently.

**Implications:** Persist job state so pause/cancel survives restarts and accurately reports pending, in-flight, completed, cancelled and failed work. Cancellation is cooperative rather than process-wide forced termination; any incomplete temporary artifacts must be safely discarded or reconciled. Do not mark an item complete until its output is durably committed. Resuming a paused job continues remaining work; a cancelled job is terminal, and a user-started rerun creates a new job while reusing valid compatible results where safe.

**Status:** User-confirmed policy; cancellation checkpoints and implementation remain to be completed.
