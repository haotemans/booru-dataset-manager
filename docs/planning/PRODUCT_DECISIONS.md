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


## D-048 — Visual analysis resource controls for a library up to 50,000 images

**Decision:** Plan and benchmark visual analysis for a target library of up to 50,000 images, not one million. Use conservative defaults that preserve desktop responsiveness, allow users to configure inference concurrency and batch size within safe bounds, and establish defaults through benchmarks on supported hardware rather than guessed hardware-specific values.

**Implications:** Keep analysis in the background and separate from ingestion. Resource settings must not compromise job persistence, pause/cancel behavior, or index integrity. Benchmark throughput, peak memory, index build time, retrieval latency and desktop responsiveness at representative library sizes up to 50,000 images. Avoid introducing distributed or million-scale infrastructure unless later evidence justifies it.

**Status:** User-confirmed target scale and resource policy; benchmark matrix, concrete defaults and implementation remain to be completed.


## D-049 — Automatic inference device selection with CPU fallback

**Decision:** Automatically detect supported GPU acceleration for DINOv3 inference and use it when runtime/model initialization succeeds; otherwise fall back to CPU. GPU presence alone is insufficient: validate runtime compatibility and successful model initialization before selecting the accelerated path.

**Implications:** Treat out-of-memory and runtime/device failures as recoverable device failures where safe. Preserve already committed analysis results, avoid marking incomplete outputs as complete, and retry the affected work on CPU when feasible. Report the selected device and fallback reason in diagnostics/status. CPU fallback must remain bounded and must not block ingestion or unrelated desktop operations. Device/runtime support varies by packaged backend and hardware; implement against the selected runtime's actual capabilities rather than assuming universal GPU support.

**Status:** User-confirmed policy; runtime-specific detection, fallback behavior and benchmarks remain to be completed.


## D-050 — Application-managed DINOv3 model downloads

**Decision:** The application downloads DINOv3 model artifacts separately from the application installer. Users may choose the model storage directory; the application provides a sensible platform-appropriate default under the user's application data area.

**Implications:** Users should not have to manually download or place model files. Display the configured model path and allow it to be changed. Download to temporary files and verify integrity using trusted checksum/release metadata before making a model active. An interrupted or invalid download must not be considered usable, and a previously working model remains available until a replacement is verified. Keep artifact/version identity tied to runtime compatibility and versioned embedding migration.

**Status:** User-confirmed policy; artifact source, checksum mechanism, path-selection UX, recovery behavior, implementation and tests remain to be completed.


## D-051 — Separate application configuration file

**Decision:** Store application-level settings in a dedicated configuration file, separate from the SQLite database that holds library and business data.

**Implications:** Keep user preferences and operational settings (for example, configured directories and safe runtime defaults) in a human-inspectable configuration format. Provide documented platform-appropriate config location and sensible defaults. Validate settings at startup and handle missing, malformed or outdated configuration safely by using defaults and surfacing actionable diagnostics rather than corrupting library data. Secrets, if any are introduced, should not be stored as plaintext in the config file; use an appropriate OS credential store. SQLite remains the source of truth for library records, media metadata and persisted job/business state. Exact format, schema versioning and migration behavior remain implementation details to define.

**Status:** User-confirmed storage approach; config format, path, schema migration, validation and implementation remain to be completed.


## D-052 — Default media directory with user-selectable location

**Decision:** On first launch, automatically use a recommended media-library directory rather than blocking startup on a directory-selection prompt. Users can choose a different location during initial setup or later in settings.

**Implications:** Keep the media library separate from application configuration and, where practical, the SQLite database so large media collections do not unnecessarily consume the system/application configuration area. Clearly display the active media path. Changing the configured path must not silently move, delete or orphan existing media. Provide an explicit, validated migration/move workflow if moving an existing library is supported; otherwise explain that selecting a different directory affects future imports or requires a deliberate library-relink operation. Handle unavailable drives and missing directories with actionable errors and recovery options.

**Status:** User-confirmed default behavior and directory configurability; exact platform-specific path, first-run UX, library migration semantics and implementation remain to be completed.


## D-053 — Unified user-selected application root directory

**Decision:** Support a user-selected root directory on one drive for the application's managed files, with separate subdirectories for application files, configuration, model artifacts, SQLite/library state, media and exports. This is the preferred managed-layout option for users who want the project and its data together. The application must not require all content to be stored on the system drive.

**Implications:** Use a clear directory layout (for example, `app/`, `config/`, `models/`, `data/`, `media/` and `exports/`) while keeping these concerns logically separate. Do not overwrite or delete user data during application upgrades. Clearly distinguish files that can be downloaded/rebuilt from primary media, database state and user decisions that require preservation. Validate available disk space and write permissions where practical; report unavailable drives and storage failures without corrupting the library. Backups and any future root-directory relocation must account for database consistency, active jobs, model/index versions and media paths. Keep the existing ability to select a different model directory or media location available when needed, but do not force users to distribute content across drives.

**Status:** User-confirmed preference for a unified root directory; exact install/update mechanism, default root, backup policy and relocation workflow remain to be completed.


## D-054 — Choose the unified root directory during installation

**Decision:** The installer asks the user to choose the application's managed root directory. The application uses that root on first launch rather than first placing managed files in a default location and requiring later relocation.

**Implications:** Validate that the selected location is writable and has sufficient available space where practical. Explain that the root will contain separate subdirectories for application files, configuration, model artifacts, database/library state, media and exports. Do not silently fall back to another drive or system-directory location if the selected drive is unavailable; present an actionable error and allow the user to reconnect the drive or choose another location. Keep upgrades from overwriting or deleting managed user data. If the installer and runtime packaging impose platform-specific constraints on where executable files can live, preserve the user-selected managed-data root and clearly document any required distinction rather than silently changing the user's choice.

**Status:** User-confirmed installation UX; installer technology, path validation, missing-drive behavior and implementation remain to be completed.


## D-055 — Uninstall defaults to preserving user data

**Decision:** Uninstalling the application removes program files but preserves user data by default. The uninstaller offers an explicit, opt-in choice to delete managed application data as well; that choice is unchecked by default.

**Scope and safeguards:** The data-removal choice must clearly identify the categories and root directory it will affect (including the database/library state, media, downloaded models, configuration and exports as applicable). Do not delete data outside the managed root or independently configured locations without explicitly identifying them and obtaining consent. If the root or a configured data location is unavailable, report that data could not be removed rather than implying cleanup succeeded. Keep uninstall and data deletion as distinguishable actions, and never make data deletion the implicit consequence of uninstalling.

**Status:** User-confirmed product behavior; installer/uninstaller implementation and exact platform-specific confirmation UI remain pending.


## D-056 — In-app update checking with user-confirmed installation

**Decision:** The application includes an update-checking entry point. When a newer release is available, it shows the version and release notes and lets the user choose to open the official release page to download the installer. The first release of this feature does not silently download or install updates and does not automatically replace the running application.

**Security and UX requirements:** Use the project's official release source; show the current and available versions and relevant release notes; handle offline, rate-limit, malformed metadata and unavailable-release cases without blocking core application use. Do not execute an installer or change installed files without an explicit user action. Verify that displayed download links point to the configured official project release location. Automatic download/install and signature verification workflows are future scope, not implied by this decision.

**Status:** User-confirmed product direction; release metadata format, update-check frequency, implementation and installer integration remain pending.


## D-057 — Automatic database and metadata backups

**Decision:** The application periodically backs up the database and critical library metadata automatically. The initial policy is one backup per day, retaining the seven most recent successful backups. Provide a manual “Back up now” action and a user-accessible restore flow as well.

**Scope:** Back up the database and the metadata needed to reconstruct library state, task state and relationships. Do not duplicate media originals or model files in each routine backup; those files can be large and should be handled separately. Store backups in a dedicated location under the managed data root by default, while making the location discoverable and configurable if supported.

**Safety requirements:** Use a database-consistent snapshot/backup mechanism rather than copying a live database naively. Never overwrite the only valid backup with a failed backup. Validate backup completion and report failures clearly. Restore must explain which current state will be replaced, preserve a recoverable pre-restore snapshot where practical, and not silently delete media or model files. Backup retention must delete only backups created and managed by this application. Exact scheduling behavior, backup format, compatibility policy and implementation remain to be specified.

**Status:** User-confirmed product direction; implementation pending.


## D-057 — Automatic database and metadata backups

**Decision:** The application automatically backs up the database and essential library/task metadata once per day, retaining the seven most recent successful backups. Provide a manual “Back up now” action and a restore entry point as well.

**Scope:** Routine backups do not duplicate original media files or downloaded model artifacts. The backup must contain enough database and metadata state to restore the library's records and task state consistently, and must use a database-consistent snapshot mechanism rather than copying a live database unsafely. Write backups atomically where practical, validate backup completion, and never replace a known-good backup with a partial one. Surface backup failures and the timestamp of the last successful backup in the UI.

**Storage and restore safeguards:** Keep backups in a designated backup location under the managed root by default, while making the location visible. Retention removes only older application-created backup files and must not delete user media or exports. Before restoring, explain that current database/library state will be replaced, require explicit confirmation, and create a safety backup of the current state where feasible. Clarify that media and model files are not included, so a metadata restore alone cannot recover deleted originals.

**Defaults:** Daily schedule; retain seven most recent successful backups. Exact scheduling behavior when the application is not running, backup format, storage limits and implementation remain to be specified.

**Status:** User-confirmed product direction; implementation pending.


## D-057 — Scheduled metadata backups with bounded retention

**Decision:** The application automatically backs up its database and critical library/task metadata on a daily schedule, retaining the seven most recent successful backups by default. Provide a manual “Back up now” action and a documented restore flow as well.

**Scope:** Routine backups do not duplicate original media files or downloaded model artifacts, which can be large and are managed separately. The backup must contain enough database and metadata state to restore library records and supported application state, and must identify its format/schema version. Use a database-consistent backup method; do not copy a live SQLite database in a way that risks an inconsistent snapshot. Retention applies only to recognized, successfully completed backups created by the application, and a failed backup must not replace the last known-good backup.

**Operational safeguards:** Store backups in a clearly identified location under the managed root by default, while allowing a future configurable destination. Surface last-success time, failures, and available space; handle a missing/unwritable drive without blocking normal application use. Restoration must validate the backup before replacing current state, explain that media and model files are not included, and preserve a recoverable copy of current database state before destructive replacement where feasible. A backup is not considered a full media-file disaster-recovery solution.

**Status:** User-confirmed direction; backup format, scheduling behavior while the application is closed, restore UX and implementation remain pending.


## D-057 — Automatic backups must restore the complete library and edit history

**Decision:** Use automatic periodic backups, with a default daily schedule and a default retention of the latest 7 successful restore points. A backup must be sufficient to restore the library's images and user changes, not merely the SQLite database.

**Backup scope:** Include a consistent snapshot of the database and required metadata, original source images, all managed derived/edited image files (including inpainting/watermark-removal results and crops), and the edit/variant relationships and history needed to restore which files came from which originals and what changes were made. Include application-managed sidecar metadata and manifests needed to reconnect records to files. Any files that are intentionally excluded must be clearly identified and must not be represented as fully backed up.

**Integrity and recovery:** A backup is not considered successful until its manifest and required files are written and verified. Use a consistent snapshot strategy so database records cannot refer to missing backup files. Provide a restore workflow that validates the backup before changing active data, reports missing/corrupt files, and avoids overwriting the current library without explicit confirmation. Preserve original images and edited variants; restoring must not flatten edits or discard the edit history. Support manual backup and restore in addition to the automatic schedule.

**Storage policy:** Because complete image backups can be large, expose the backup destination and estimated/required space where practical. Do not silently delete source images, edited files, or restore points to make room. Retention cleanup may remove only older verified backup sets beyond the configured retention count, and must never remove the active library files.

**Status:** User-confirmed requirements; backup format, storage destination, scheduling implementation, consistency strategy and restore UX remain to be designed and implemented.


## D-058 — One local restore point with remote backup archival to ModelScope

**Decision:** Keep only one latest verified full restore point locally. Send older restore points to ModelScope (魔搭) as remote backup archives, subject to successful upload and integrity verification.

**Safety invariants:** Create and verify a new full restore point before replacing the previous local restore point. Never delete the previous local point until the new point is complete and verified. Never remove a local older backup until its remote upload has completed and the remote archive can be verified against its manifest/checksums. If authentication, connectivity, quota, upload, or verification fails, retain the local data and report that remote archival is incomplete; do not silently discard backups. Show upload and retention status to the user.

**Restore scope:** Each full restore point must contain original images, derived/edited images (including inpainting/watermark-removal results and crops), database/library state, sidecar metadata, and provenance/version relationships required to restore edits and associate each variant with its source. Restore must validate the archive before replacing active library state and require explicit confirmation before destructive changes.

**Configuration and limits:** The ModelScope destination/repository, authentication method, privacy/visibility, quota and large-file/upload constraints must be confirmed and implemented before relying on remote backups. Do not hard-code credentials in application configuration or logs. Make clear that remote backup is not complete until verification succeeds. If ModelScope is unavailable or unsuitable for the required file sizes or privacy expectations, preserve local backups and ask the user to resolve configuration rather than falling back to destructive cleanup.

**Status:** User-confirmed retention and destination direction; ModelScope integration, secure authentication, archive format, upload/resume behavior, and remote restore remain pending implementation and feasibility validation.


## D-059 — Use a private ModelScope repository without application-level backup encryption

**Decision:** Do not add a separate application-level encryption layer to backup archives. Store remote backup archives in a private ModelScope (魔搭) repository, relying on the repository's access controls.

**Security requirements:** Explain that private-repository access control is not the same as end-to-end backup encryption. Keep access tokens out of source code, ordinary configuration files, logs, crash reports and backup archives; use an appropriate OS credential store where available. Request only the permissions needed for backup upload and restore. Verify repository visibility before uploading, and clearly report if the destination is public or its privacy cannot be confirmed. Remote restore must authenticate before accessing private backup content.

**Status:** User-confirmed decision; secure credential storage, private repository verification and ModelScope integration remain pending implementation and validation.


## D-060 — Resumable ModelScope uploads and post-verification local cleanup

**Decision:** Use resumable uploads for remote backup archives where supported by the selected ModelScope upload mechanism. After an older restore point has uploaded successfully and remote integrity has been verified, delete its temporary/local archive copy so local storage retains only the latest verified full restore point.

**Safety invariants:** Never delete an upload source while the upload is in progress, after an interrupted/failed upload, or before the remote object and its manifest/checksums have been verified. On interruption, preserve the partial-upload/resume state and local source so upload can continue. If resume is unsupported by the actual ModelScope interface, safely restart or retry without deleting the only remaining valid copy. Always keep the latest verified local full restore point; only clean up older backup archives after verified remote archival. Report upload, verification, and cleanup outcomes separately, and do not report cleanup success unless deletion actually completed.

**Status:** User-confirmed behavior; resumability depends on the selected ModelScope upload API/client and remains to be implemented and tested.


## D-061 — Incremental, content-addressed backups with safe garbage collection

**Decision:** Avoid uploading a full copy of the image library for every backup. Use a manifest-based incremental backup design with content hashes and deduplicated immutable file objects. Each restore point is a complete logical snapshot of database/library state, source images, derived edits and provenance, but references previously uploaded unchanged objects instead of re-uploading them. Upload only newly added or changed objects plus the new snapshot manifest/database delta required by the implementation.

**Restore semantics:** A restore point must be independently identifiable and fully restorable by resolving its manifest and all referenced objects, including any required prior deltas. The system must validate object hashes, manifest completeness and database consistency before declaring the restore point valid. Prefer periodic self-contained metadata checkpoints or a bounded delta chain so restoring does not require an unbounded sequence of replay steps. Preserve original images and all edited variants, including inpainting/watermark-removal outputs and crops, as well as the relationships and edit history needed to reconnect them.

**Deletion and garbage collection:** Removing a library item or an obsolete restore point must not immediately delete shared remote objects. Compute reachability from all retained restore points and the current library; delete only objects that are unreferenced by every retained restore point, and only after a safety grace period and a verified inventory. Use an explicit mark-and-sweep or equivalent transactional process, protect the latest verified local restore point, and never collect objects while an upload, restore, or manifest update is in progress. If reachability/inventory is uncertain, retain data and report cleanup as incomplete. Remote restore-point retention policy remains configurable; deletion of old snapshots may reclaim objects only when no retained snapshot references them.

**Upload and recovery:** Uploads should be resumable where supported, verify each object after upload, and commit a snapshot manifest only after all referenced objects are present and verified. Interrupted uploads must not create a valid-looking complete restore point or trigger cleanup of the only valid local copy. Backups remain unencrypted at the application layer and target a private ModelScope repository as separately decided; actual ModelScope object, quota and API capabilities must be validated before implementation.

**Status:** User-confirmed design direction; object layout, database snapshot/delta format, ModelScope API feasibility, restore-chain limits, remote garbage collection and failure-injection tests remain to be designed and implemented.


## D-062 — Tiered retention for remote incremental restore points

**Decision:** Use tiered retention for verified remote restore points in the private ModelScope repository:
- Keep one daily restore point for each of the most recent 7 days.
- Keep one weekly restore point for each of the most recent 4 weeks.
- Keep one monthly restore point for each of the most recent 12 months.

When a daily, weekly or monthly tier overlaps, a single restore point may satisfy multiple tiers. Prefer the most recent verified restore point within the relevant period. Retention applies to logical restore points, while content-addressed file objects may be shared across snapshots.

**Deletion safety:** Remove an obsolete restore point only after confirming that it is outside all retention tiers and not otherwise pinned by the user. Then garbage-collect an object only if no retained restore point or active library state references it, and only after a safety grace period and verified inventory. The latest verified local restore point must remain protected. If the system cannot establish reachability or upload/restore status, retain the data and report cleanup as incomplete.

**Status:** User-confirmed retention policy; scheduler, tier selection, user-pinned restore points and reference-safe garbage collection remain pending implementation and testing.


## D-063 — Back up edited image results; re-fetch source originals when possible

**Decision:** The backup set prioritizes user-created/edited image outputs rather than duplicating source originals that can be downloaded again from Danbooru. Preserve edited results such as inpainting/watermark-removal outputs, crops and other derived variants, plus the metadata and provenance needed to associate each result with its source and restore the edit/version relationships. Source-original image files are excluded from remote backup by default.

**Source recovery metadata:** For every excluded source original, preserve stable source identifiers and the best available source URL/provider, source post ID, expected file hash, relevant file metadata, and the parent/variant relationship. Restore should clearly mark source originals that need to be re-fetched and should not treat a missing source download as a reason to discard a backed-up edited result. Re-fetch only from the recorded source and validate the retrieved file against the stored hash when available.

**Limitations and safety:** Danbooru availability, post deletion, access restrictions, changed URLs or source-file changes can make an original impossible to retrieve later. The application must disclose that source originals are not covered by remote backup and report unrecoverable originals during restore. If a source original is no longer available, retain and restore the edited output and its metadata; do not silently claim a fully identical reconstruction of the original library. Keep the local latest restore point policy consistent with this scope, and do not delete a local original unless the user explicitly chooses that storage policy separately.

**Status:** User-confirmed backup scope; exact manifest schema, source re-fetch workflow and missing-source reporting remain pending implementation and testing.


## D-064 — Back up committed edit outputs and reproducible edit parameters

**Decision:** For each user-committed edit, back up the resulting image output and the parameters needed to understand or continue the edit. Do not back up transient previews or uncommitted intermediate operations by default.

**Required edit record:** Preserve the parent/source variant reference, output file hash and metadata, operation type (for example inpainting or crop), operation parameters and coordinates, relevant mask or selection assets when required to reproduce/continue the edit, and tool/model/version identifiers where available. Record provenance and timestamps as appropriate. Store enough information to distinguish an edit that can be reproduced from one whose output is preserved but whose exact regeneration is no longer possible.

**Recovery semantics:** Restore the committed output even if its original model, tool version, mask or source original is unavailable. Where dependencies remain available, present the saved parameters and provenance so the user can continue editing; do not promise deterministic reproduction if the tool/model is nondeterministic or dependencies are missing. Preserve the parent-child version graph and validate references when restoring. Failed or partial backup must not mark an edit record as backed up.

**Status:** User-confirmed decision; edit-record schema, mask/selection persistence, tool-specific parameter adapters and restore UI remain pending implementation and testing.

## D-065 — Single local image copy and seven-day remote backup cadence

**Decision:** Optimize for low local storage use and low network traffic. The application must not keep a local archive of every edited image version. For each image/variant, keep only the current committed file locally; committing a new edit replaces the previous local output rather than creating another retained image copy. Temporary previews and intermediate outputs must be cleaned up when no longer needed.

**Remote backup cadence:** Run remote backup on a seven-day interval, not daily. Use incremental, content-addressed backup and avoid uploading unchanged objects again. Keep the previously agreed remote retention tiers (daily/weekly/monthly restore points) only to the extent they can be represented by deduplicated objects; this retention policy does not authorize keeping multiple local image copies. A restore point is complete only after its manifest and all referenced objects have been uploaded and verified.

**Trade-off:** Because the local previous output is overwritten and remote backup runs weekly, an edit made after the latest successful remote backup may not be recoverable if overwritten or damaged before the next successful backup. The application must communicate the last successful backup time and pending/unprotected changes; it must not silently claim every edit is recoverable. Do not add local version archives to avoid this trade-off unless the user explicitly changes this decision.

**Status:** User-confirmed storage and cadence requirements; scheduling, status UI, overwrite behavior, and backup verification remain pending implementation and testing.

## D-066 — Re-edit from the source instead of relying on image-version backups

**Decision:** An ordinary editing mistake should be recoverable by re-fetching the source original from its recorded Danbooru post and starting the edit again; it should not require a backup restore. Keep only the current local image output per image/variant and do not retain multiple local image-version files solely for undo/history. Preserve the source post ID/URL, expected source hash when known, and edit parameters/metadata needed to re-fetch and re-run the workflow.

**Recovery behavior:** Provide a direct action to re-fetch the source original and restart editing. Validate the downloaded source against the recorded hash when available. If the source cannot be fetched or no longer matches, explain the failure and offer available recovery paths; do not silently overwrite the current output before the source is successfully fetched and validated. This is a recovery workflow, not a guarantee that every remote source will remain available.

**Backup role and cadence:** Remote backup remains weekly as separately decided. It protects against loss of data that cannot simply be re-fetched or regenerated, rather than serving as the normal undo mechanism for editing mistakes. Do not increase upload frequency or add local image-version archives to solve routine edit mistakes.

**Status:** User-confirmed recovery principle; source re-fetch action, safe replacement flow and validation remain pending implementation and testing.

## D-067 — Retain edit parameters and masks, not historical output images

**Decision:** Retain the parameters needed to repeat or continue an edit, including inpainting masks/selections, watermark-removal regions, crop coordinates, operation settings, and available tool/model/version identifiers. Keep these as compact edit metadata/assets associated with the current image variant; do not retain multiple historical output image files locally.

**Re-edit workflow:** When the user chooses to re-edit, re-fetch the source original from its recorded Danbooru source, validate it when a source hash is available, and offer to reuse the saved parameters/masks as the starting point. Because masks and coordinates may depend on image dimensions or alignment, validate compatibility before applying them and allow adjustment if needed. Do not overwrite the current output until the source is fetched and validated and the user confirms the replacement.

**Storage and backup:** Avoid duplicating unchanged parameter assets unnecessarily. Include the edit parameters and necessary masks in the weekly remote backup so a source image can be re-fetched and the edit workflow reconstructed where dependencies permit. This does not require keeping old output image versions.

**Status:** User-confirmed behavior; parameter schema, mask persistence, compatibility validation and re-edit UI remain pending implementation and testing.

## D-068 — Preview before replacing the current edited output

**Decision:** Re-editing must produce a preview first. Do not replace the current committed output until the user explicitly confirms the preview. Canceling or rejecting the preview leaves the current output unchanged. Temporary preview files are disposable and must be cleaned up after confirmation, cancellation or failure when no longer in use; they are not retained as historical image versions.

**Safety:** Validate the new output and ensure it is readable before replacement. Perform the final replacement atomically where supported so an interrupted write does not leave a partial output. Keep the current output until the replacement is ready and confirmed.

**Status:** User-confirmed behavior; preview lifecycle, atomic replacement and cleanup remain pending implementation and testing.

## D-069 — Unified image editing tools in the review page

**Decision:** Integrate image editing into the existing review page rather than creating separate editing screens. Provide a unified editing workspace there, with selectable tools such as watermark removal and inpainting, and support previewing the result before the user confirms replacement of the current output.

**Workflow:** Keep the review context visible while editing so the user can assess the image and decide whether to accept the result. Reuse the existing source re-fetch, saved edit parameters/masks, preview lifecycle and safe replacement rules. Do not create extra retained image versions as part of the editing workflow.

**Status:** User-confirmed direction; review-page layout, tool switching, editor integration and interaction details remain pending implementation and testing.

## D-070 — Persistent image canvas with a right-side editing panel

**Decision:** In the review page, keep the image visible in the main/left canvas and place editing tools and their controls in a right-side panel. Switching between tools (for example watermark removal and inpainting) must not navigate away from the image or hide the review context.

**Interaction:** The right panel contains the selected tool's controls and actions. Show the generated result as a preview in the image area; keep the current committed output unchanged until the user explicitly confirms replacement. Preserve usable image visibility while adjusting parameters, and make the layout adapt to smaller windows without losing access to the image or tool controls.

**Status:** User-confirmed layout direction; responsive behavior, tool panels and preview integration remain pending implementation and testing.

## D-071 — Zoom and pan in the review editor canvas

**Decision:** The review-page image canvas supports zooming in/out and panning by dragging, allowing precise inspection and selection for watermark removal, inpainting and other editing tools. Provide a way to return to a fit-to-view or reset view.

**Interaction constraints:** Zoom and pan change only the canvas viewport, not the source pixels or edit output. Tool selections, masks and coordinates must map correctly between the displayed viewport and the original image coordinate space, including after zooming, panning or resizing the window. Keep the image and right-side tool panel available throughout editing.

**Status:** User-confirmed behavior; canvas controls, coordinate transforms and input handling remain pending implementation and testing.

## D-072 — Manual and AI-assisted region selection

**Decision:** The review editor supports rectangle, freehand brush and polygon selections, and is not limited to manual selection. Integrate AI visual-recognition assistance through an appropriate tool/model interface so the user can request candidate regions for tasks such as watermark detection/removal and inpainting. AI suggestions assist the user; they do not silently commit edits or replace the current image.

**Workflow:** Users can create or adjust selections manually, request AI-generated candidate masks/regions when supported, inspect and refine the proposed selection, and then preview the edit. Keep selection coordinates aligned with the original image regardless of canvas zoom/pan. Tool/model availability and task suitability should be explicit; if AI assistance fails or is unavailable, manual selection remains usable.

**Architecture:** Keep visual-recognition providers behind a tool/adapter interface rather than hard-coupling the editor to one model. Record the selected tool/model and relevant parameters when available so an edit can be continued or reproduced as far as dependencies allow. Do not invoke remote AI services without making the action and its applicable configuration clear to the user.

**Status:** User-confirmed direction; provider/tool adapters, AI region proposals, mask editing and preview integration remain pending implementation and testing.

## D-073 — User-selectable local and API visual-recognition providers

**Decision:** Support both local visual-recognition models and configured AI API providers for editing assistance. Make the provider choice configurable in application settings. Default to local recognition where a suitable installed model is available, to reduce network traffic and API costs; allow users to choose a configured API provider when desired or when local capability is insufficient.

**Behavior:** Clearly identify which provider will be used before a task runs. Do not silently fall back from local to a remote API, because that can incur network traffic or cost; ask for user consent or require an explicit fallback setting. If the selected provider is unavailable, explain the issue and keep manual editing available. Provider-specific credentials must use the application's secure credential handling and must not be written to ordinary logs or configuration files.

**Status:** User-confirmed direction; provider settings, local model availability checks, provider adapters and explicit fallback behavior remain pending implementation and testing.

## D-074 — Confirm AI-detected watermark regions before inpainting

**Decision:** When AI identifies a suspected watermark, first display the proposed region/selection for user review. The user must confirm or adjust the selection before the application generates an inpainting repair preview. The resulting image preview must still be explicitly confirmed before replacing the current committed output, as specified in D-068.

**Behavior:** AI detection never directly triggers inpainting or changes the committed image. Users can edit or reject the proposed region and continue with manual selection if detection is unavailable or inaccurate.

**Status:** User-confirmed workflow; region-review UI, selection-to-mask conversion and inpainting integration remain pending implementation and testing.

## D-075 — Batch AI region detection with per-image approval

**Decision:** Support running AI-assisted region detection across multiple images. Batch detection may prepare candidate regions/selections for review, but each image's proposed region must be reviewed and confirmed individually before inpainting is generated for that image. Each repaired image must then be explicitly approved before replacing its current committed output, consistent with D-068 and D-074.

**Behavior:** Batch processing must not turn detection into unattended destructive editing. Rejected or failed items leave their current outputs unchanged; users can adjust individual selections and proceed manually. Show per-image progress and outcomes so failures do not obscure successful items.

**Status:** User-confirmed workflow; batch scheduling, per-image review states, failure handling and UI remain pending implementation and testing.

## D-076 — Use the configured visual-recognition provider

**Decision:** AI-assisted region detection uses the provider selected in application settings. If settings select a local model, use that local model; if settings select an AI API provider, use that provider. Do not ask the user to choose the provider again for every batch task.

**Behavior:** Before execution, validate that the configured provider is available and sufficiently configured. If it is unavailable or fails, report the problem and keep manual editing available; do not silently switch to a different provider or make remote calls contrary to the saved setting. Any explicit fallback policy must itself be configured in settings. Existing per-image selection review and final output approval remain mandatory regardless of provider.

**Status:** User-confirmed behavior; provider settings validation, execution routing and failure UI remain pending implementation and testing.

## D-077 — Keep the initial editing toolset focused

**Decision:** The initial review-page editing toolset is limited to:
- Watermark removal/inpainting.
- Cropping.
- Rectangular selection.
- Freehand brush selection.
- Polygon selection.

Do not expand the first release with additional image-editing tools unless a later user decision explicitly adds them.

**Status:** User-confirmed scope; implementation and testing remain pending.

## D-078 — Preserve source format for edited outputs by default

**Decision:** Save edited outputs in the source image format by default. Convert to another format only when the source format cannot support the required output or when the user explicitly chooses a different format.

**Behavior:** Do not silently perform lossy conversion. When conversion is required, explain the reason and use a suitable output format; preserve image dimensions and visual quality as far as the selected operation and format allow. The output must be validated before replacing the current committed file, following D-068.

**Status:** User-confirmed policy; format capability checks, conversion behavior and tests remain pending implementation.

## D-079 — Optional configurable edited-file size limit

**Decision:** Do not impose a default fixed file-size limit on edited images. Offer an optional configurable per-file size limit for users who want to manage disk usage.

**Behavior:** When the limit is enabled and an output exceeds it, notify the user and allow adjustment rather than silently degrading image quality or discarding the output. Any compression or format conversion must be explicit and consistent with D-078. Validate available disk space and the output file before replacing the current committed image.

**Status:** User-confirmed policy; settings UI, output-size checks and related tests remain pending implementation.

## D-080 — Rejected images are marked for deletion, with explicit confirmation

**Decision:** The default action for images rejected during review is deletion, but deletion must not occur until the user explicitly confirms it. Rejection alone must never immediately delete files.

**Behavior:** Clearly identify the rejected images and the deletion scope in a confirmation step before deleting. If the user cancels or declines confirmation, keep the images and their associated records unchanged. Avoid deleting unrelated files or data. Report deletion failures rather than treating them as successful.

**Status:** User-confirmed policy; review actions, confirmation UI and safe deletion behavior remain pending implementation and testing.

## D-081 — Batch confirmation for rejected-image deletion

**Decision:** For batch review, collect rejected images into a pending-deletion list. After the batch review, show the full list and require one explicit confirmation before deleting the listed items.

**Behavior:** The confirmation view must make the deletion scope clear. Cancelling leaves all listed images and their associated records unchanged. Do not delete items that are not in the confirmed list; report any deletion failures accurately.

**Status:** User-confirmed policy; pending-deletion list, confirmation UI and deletion implementation remain pending implementation and testing.

## D-082 — Delete rejected images and their image-specific data together

**Decision:** Once the user confirms the batch deletion, delete each listed rejected image and its image-specific database records and derived data together. This includes its tags/caption sidecars, AI analysis results and visual features/embeddings where applicable.

**Safety:** Only remove data owned by the confirmed images. Do not delete shared model files, shared indexes, or resources referenced by other images; update or rebuild shared indexes as required. Keep the operation consistent and report partial failures clearly rather than claiming full success. Before confirmation, nothing is deleted; cancelling leaves files and records unchanged.

**Status:** User-confirmed policy; dependency/reference checks, coordinated cleanup and failure recovery remain pending implementation and testing.

## D-083 — Delete confirmed rejected items immediately without an undo window

**Decision:** After the user explicitly confirms the batch deletion, execute deletion immediately. Do not add a temporary undo window or maintain a separate trash state for rejected images.

**Safety:** The confirmation step must identify the exact deletion scope. Before confirmation, no files or records are removed. The deletion implementation must coordinate image-specific files and database records, preserve shared resources, and report failures accurately.

**Status:** User-confirmed policy; implementation and testing remain pending.

## D-084 — Support grid and masonry gallery layouts

**Decision:** Support both a fixed-size thumbnail grid and a responsive masonry layout in the gallery. Use the grid as the default layout and let users switch layouts in the interface.

**Behavior:** Changing layout affects presentation only; it must not alter image files, ordering, selection, or metadata. Both layouts should retain consistent selection and review actions and adapt to the available window size.

**Status:** User-confirmed direction; layout switching and responsive presentation remain pending implementation and testing.

## D-085 — Generate and cache rebuildable gallery thumbnails

**Decision:** Generate and cache thumbnails for gallery browsing rather than repeatedly decoding and resizing full-resolution images. Treat thumbnails as disposable derived cache, not as authoritative image copies or backup data.

**Behavior:** Store thumbnails in the designated cache area, reuse them across grid and masonry layouts, and regenerate them when missing, invalid, or stale. Cache cleanup must never delete original images or committed edited outputs. Avoid creating redundant full-resolution image copies; thumbnail generation and cache management must be bounded and suitable for galleries approaching 50,000 images.

**Status:** User-confirmed direction; thumbnail pipeline, cache invalidation and cleanup remain pending implementation and testing.

## D-086 — Default to newest imports with selectable gallery sorting

**Decision:** Sort the gallery by import time descending by default, showing the most recently imported images first. Also support sorting by filename, file size, image dimensions and rating where those values are available.

**Behavior:** Sorting changes presentation/order only and must not modify image files or metadata. Keep ordering deterministic by applying a stable tie-breaker when primary sort values match. If rating is unavailable for an image, handle it consistently rather than inventing a value.

**Status:** User-confirmed direction; sort controls, query/index support and tests remain pending implementation.
