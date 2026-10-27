# AI API integration and security

Current code connects strict JSON text-only jobs, protected-name isolation, local merging, target version history, bounded retries, cooldown persistence, content recovery and explicit resume. The complete multi-profile concurrency controller below is still a specification; initial desktop dispatch is serialized. See [delivery evidence](../planning/WORKFLOW_DELIVERY.md).

## Desktop credential settings

The desktop API Configuration dialog accepts a masked API Key per profile. Keys are stored separately in Windows Credential Manager, under service `Booru Dataset Manager` and account `ai-<stable-profile-id>`. Ordinary RON settings persist the non-secret profile ID, name, base URL, model, concurrency and group. Legacy profiles receive unique IDs; profile names and list indices are never credential identifiers. Danbooru credentials use their separate entry and settings page.

Saving clears the key input; editing reports only whether a key is saved and never prefills it. Blank input retains the existing key for the same canonical base URL. Changing the URL requires a replacement key or explicitly removing the previous key. A new profile for an unauthenticated service may omit the key. The key-only trash control clears the current profile's credential; removing a profile also removes its credential. Storage failures preserve the existing metadata and report a redacted error. Cancel/close/Escape discard unsaved input. Credentials never enter logs or ordinary settings; unsupported platforms report unavailable storage rather than fall back to plaintext.

Credential save/load/delete is implemented and tested locally on Windows, including isolated synthetic native entries. This is not HTTP integration: saving makes no request, connection testing remains disabled, and no AI execution is implemented. Screenshot runs never load real profile credentials.

## Provider profiles

Support multiple named OpenAI-compatible profiles with base URL, model, timeout, bounded retries and concurrency. The default maximum concurrency is 2 requests, configurable per provider profile. All tasks using the same provider profile share this limit and its adaptive rate-limit controller; separate profiles have independent limits. Enforce the cap centrally by profile, including tag optimization and caption generation, rather than assigning each task its own allowance. Enforce the limit in the shared scheduler; batch size 8 does not mean eight requests are sent simultaneously. Validate configured concurrency within a bounded safe range and do not increase it automatically in response to throughput. Changes to the configured maximum apply immediately to future dispatches. Track the configured maximum separately from the active rate-limit-adapted limit; the effective limit must not exceed either. Increasing the configured maximum does not clear a 429-induced reduction; keep the current active limit and continue gradual recovery up to the new maximum. Changing the configured maximum does not reset the cooldown or recovery counter. Lowering the configured maximum applies to future dispatches without cancelling already in-flight requests solely because of the setting change. Gradual recovery may raise the active limit only after the configured cooldown and never above the latest configured maximum. If the user lowers the cap below the number of in-flight requests, dispatch no new requests until in-flight work falls below the new cap; do not cancel already-sent requests solely because of this setting change. Default request timeout is 120 seconds, with at most one retry per request. Apply error-specific retry rules: transient connection failures/timeouts and eligible HTTP 5xx responses may be retried once with bounded backoff; HTTP 429 should honor `Retry-After` when present but must not exceed the one-retry limit; after the cooldown, retry the same request once. If that retry still fails, persist the affected item IDs for later recovery and continue processing subsequent items. A 429 immediately sets the affected provider profile's active concurrency to 1 and resets its cooldown; unresolved IDs remain resumable. Keep the configured serial request-start interval in force; On HTTP 429, set the affected provider profile's active concurrency limit to 1 and dispatch requests serially for that profile; do not send parallel requests while this degraded mode is active. All tasks using the same provider profile share one rate-limit scheduler and cooldown gate: a 429 from any task blocks new dispatch for all tasks on that profile until the cooldown expires. Other provider profiles remain independent. Persist profile-level cooldown deadline and active concurrency state so pausing, resuming, or restarting the app cannot bypass the gate. Store the absolute deadline; after restart, wait only for the remaining time, or resume if the deadline has passed. If system clock changes make the deadline untrustworthy, fail conservatively by retaining serial mode and re-establishing a safe cooldown rather than raising concurrency unexpectedly. This supersedes any halve-current-limit behavior. For recovery, honor a server-provided `Retry-After` as the cooldown when present; if absent, use a default cooldown of 5 minutes. Once the cooldown elapses without another 429, restore concurrency gradually by increasing the active limit by 1 at a time, never above the latest user-configured profile maximum. Any new 429 resets the cooldown and returns the active limit to 1. After cooldown, require 5 consecutive successful serial requests by default before raising the limit from 1 to 2; each subsequent increase requires another 5 successful requests at the current active limit. Both success-count thresholds are configurable. Persist recovery progress across restarts. A new 429 resets recovery progress; failed requests and items pending network recovery do not count as successful requests. While serial mode is active, enforce a configurable minimum interval between request starts, defaulting to 1 second. Measure the interval from the start of one request to the start of the next; if the previous request took at least the configured interval, add no extra post-completion delay. This pacing interval is separate from the cooldown and remains in force while concurrency is 1. Restore concurrency gradually only after a defined cooldown with no further 429 responses. Honor `Retry-After` when present. A 429 does not trigger batch shrinking or content-processing fallback; authentication/authorization and clearly non-transient client errors are not automatically retried. A failed transport retry must not trigger batch shrinking or the content-processing overlap rule. Persist unresolved item IDs for later recovery and never retry indefinitely. Keep separate profile/feature switches for tag optimization and caption generation. Test-connection and model-list operations must return redacted diagnostics. Never commit API keys or write them to logs, crash reports, backup manifests or ordinary settings files; use an OS credential store where available and clearly explain any fallback limitations.

Artist and character canonical names are removed from the AI rewrite input and merged back locally. Protected terms and aliases are passed as constraints only when needed; unknown terms remain unchanged and flagged for review. Raw tags are immutable provenance. Caption generation must be factual and avoid unsupported visual claims.

## JSON request and response contract

All AI API requests must be serialized as JSON. All accepted operation results must be parsed and validated as JSON; free-form prose, Markdown fences, or text scraping are not valid success responses.

- Define a versioned schema per operation (visual assessment, crop proposal, tag optimization, caption generation). Include explicit operation/version, stable item ID, required fields, types, enums, and numeric/string/array bounds.
- For batch requests, give every input item a stable unique ID and require each result to identify that same ID. Validate exact correspondence: unexpected, duplicate, missing, or mismatched IDs are item-level failures; never join responses to inputs solely by array order.
- Use provider-native structured output or JSON Schema mode where supported. Maintain provider capability flags. If unavailable, still request strict JSON and use the same local parser/validator; do not claim the provider enforces a schema when it does not.
- Parse with a real JSON parser, then validate schema and domain invariants. Reject malformed JSON, wrong types, out-of-range values, invalid crop rectangles, and unsupported status values. Do not silently repair a response into a success.
- On a failed request/window, do not retry the same batch and do not split it into smaller requests. Record the failure and advance the input window according to the sliding-window policy below.
- If a syntactically valid response contains valid results for some items and invalid/missing results for others, persist the valid item results; mark only affected items as failed. Treat the window as failed for cursor advancement, without resending it. Stable item IDs and idempotent writes prevent overlap from duplicating results.
- Persist job and per-item states so interrupted jobs can resume without resending already accepted work. Repeated results must not overwrite human-reviewed choices or create duplicate accepted outputs.
- Record provider/model, schema version, request/job ID, attempt count, validation outcome and error category. Keep diagnostics redacted; cap raw response retention and never log image bytes, secrets or credential-bearing headers.
- JSON validity is not semantic correctness. Apply protected-term checks and domain validation; vision-based aesthetic and crop proposals remain advisory and require human review.

The caption vision path uses one JSON batch request. Each image is decoded and transformed in memory immediately before dispatch: EXIF orientation is applied, transparency is composited onto white, the aspect ratio is preserved, and small images are never enlarged. The configured longest edge is capped at 1024 pixels; JPEG quality is reduced and the image is resized again if needed so the encoded image bytes stay within the 1 MiB upload budget. The original file is never modified. If one image cannot be read or encoded, that ID is deferred and the remaining valid items still form a request. Responses are parsed locally with the versioned JSON schema and correlated by stable IDs.

## Batch sizing and sliding-window policy

- Default to 8 items per API request; supported fallback batch sizes are 4, 2 and 1.
- Distinguish transient transport failure, batch-processing failure, and item-level partial failure. Network timeouts and connection interruptions are transient transport failures: retry the same batch with backoff and do not shrink the batch because of these errors. Malformed JSON or response-level schema failure are batch-processing failures; item-level failure means a valid response identifies failed items or omits expected item results.
- If bounded retries for a transient network failure are exhausted, record affected IDs as deferred transport failures and continue the main scan. Do not shrink the batch or classify these as content failures. Queue them for post-scan recovery; if connectivity is still unavailable, keep them pending for a later resume.
- On a fully successful 8-item window, advance the main cursor by 8. If a batch-processing failure occurs, do not resend the identical payload indefinitely; advance the main cursor by 4, producing a 4-item overlap. Network timeouts and connection interruptions do not trigger this shrink/overlap rule: retry the same batch with backoff.
- For a valid partial response, persist all valid item results and enqueue only failed/missing item IDs as a recovery group. The normal 8-item main window still advances by 8; do not reprocess successful items as part of recovery.
- Continue scanning main windows while collecting failed groups. Do not dispatch recovery requests until the main scan reaches its end; then drain the recovery queue in queue order. Recovery work must not block normal main-window progress.
- Deduplicate queued recovery work by stable item ID across all overlapping windows and failure groups. Maintain at most one pending recovery entry per item, while preserving separate failure/audit events as needed. Immediately before dispatch, re-check persisted item state and skip items that already have an accepted valid result or human-reviewed content.
- After the main scan completes, regroup deduplicated pending recovery items in queue order into requests of at most 8 items. The final request may be smaller; never add padding entries. Preserve stable item IDs and links to the originating failure events.
- In recovery batches, persist valid partial results and requeue only failed/missing items. If the request has a transient network timeout or connection interruption, retry the same batch with backoff and do not shrink it. If a batch-processing failure occurs, split only that failed batch using the bounded 8 → 4 → 2 → 1 fallback policy.
- Recovery is a separate fallback phase, not a same-payload, same-size retry. If a recovery group fails, subdivide only that failed group progressively (e.g. 2 failed items become two single-item requests); preserve previously accepted results. Never retry an identical failed request indefinitely.
- Batch-processing failures advance the main cursor by 4 and create overlap. Transient network failures do not advance by 4 or create overlap solely due to the transport error; retry the same batch with backoff. Keep unresolved item IDs tracked so failures cannot silently drop work.
- At a tail that cannot advance further at the current window size, reduce batch size progressively: 8 → 4 → 2 → 1. At each fallback level, move by that level's batch size: 4-item windows advance by 4, 2-item windows by 2, and single-item windows by 1. A failed 4-item fallback group is subdivided into 2-item groups; a failed 2-item group is subdivided into single-item groups.
- A failed single-item request is recorded as failed/deferred for later manual handling; do not retry indefinitely.
- Track stable item IDs and make writes idempotent. Overlapping main windows and recovery requests must not duplicate accepted results or overwrite human-reviewed data.
- Persist the cursor, fallback level, job state and item outcomes. Restart/resume must not re-send accepted work solely because the previous process stopped.
- On application startup, detect incomplete persisted jobs and present their saved status/progress for user-confirmed resumption. Do not dispatch any external API request automatically on startup; resumption begins only after explicit user action. Dismissing the prompt leaves the job resumable from the task UI.
- After the user confirms resume, reconcile requests that were in flight when the process stopped before resending them. Prefer provider-supported request-status lookup and/or idempotency keys with a stable persisted request ID. Do not assume provider support: if neither capability exists, use a documented bounded fallback, record ambiguous outcomes, and avoid indefinite resend loops. Exactly-once execution cannot be guaranteed without provider support.
- A job is not complete while any item remains pending due solely to transient network failure. Keep the job resumable and preserve those item IDs in the recovery queue; resume recovery when connectivity returns. Completion requires every input item to have a successful result or an explicit terminal failed/deferred outcome after applicable content-processing fallbacks.
- The final window may contain fewer items than the current batch size. Never fabricate padding entries. A job is complete only when every input item is completed or explicitly recorded as failed/deferred after the applicable fallback processing.


## External vision API upload consent

Image analysis is a separate capability from text-only AI operations. External image upload is disabled unless the user explicitly approves the current batch.

Before dispatching a vision request, the application must:
1. Prepare the exact candidate batch and show the configured provider/profile, image count, analysis purpose, and any available cost/usage estimate.
2. Explain that image content will be transmitted to that external provider. Identify provider privacy/retention information separately when available; do not invent or guarantee provider behavior.
3. Let the user inspect the batch and remove items, approve the batch, or cancel.
4. Bind approval to that batch/job and purpose. Do not persist it as global consent or reuse it for later batches.
5. Block dispatch until approval is recorded. Cancellation, missing UI confirmation, or unavailable consent state means no upload; report the job as awaiting approval or cancelled.
6. Record a minimal consent audit event (timestamp, provider/profile identifier, job/batch identifier, scope/count, decision) without storing image bytes, credentials, or unnecessary sensitive content.

Background jobs may prepare a batch, but must pause before sending it until the user approves. Do not treat an earlier approval as authorization for a different batch or different purpose. Local-only analysis can proceed without external upload consent when a supported local analyzer is explicitly selected.

Never send API keys, cookies, provider session tokens, browser-session data, or unrelated local files to the vision endpoint. Do not log request bodies containing image data or credential-bearing headers. Redact diagnostics and ensure cancellation prevents not-yet-dispatched requests where possible.

## Vision assessment and crop suggestions

A vision model may assess aesthetic completion, visual clutter and visible text, and may propose crop coordinates with a rationale. A crop is only a derived proposal. Preserve original bytes and SHA-256; show original and proposed crop side by side. Require human approval or adjustment before using a crop in export. Store provider/model/prompt version, crop parameters, response status and review outcome. Treat model output as untrusted and validate schema, coordinates, bounds, aspect ratio and output size.

## Output validation and safety

Treat model output as untrusted data. Validate schema, length, encoding, protected-term coverage, unauthorized additions, and output target. Do not allow model output to choose filesystem paths, SQL fragments, shell commands, or remote repository names. Use bounded request/response sizes and cancellation. A failed target must not overwrite its last valid version or undo another target's successful result.

Acceptance tests: JSON serialization, strict JSON parsing, schema validation, unknown/duplicate/missing batch item IDs, malformed and oversized output, no same-window retry, no batch splitting, partial batch result preservation with failed-item-only queued recovery after the main scan, normal main-window advancement, sliding-window advancement, queue persistence and resume, recovery queue deduplication by stable item ID, skip already-successful or human-reviewed items, ordered regrouping into recovery batches of up to 8 (full success advances 8; failed non-tail window advances 4), tail fallback and stride (8 → 4 → 2 → 1), failed-window subdivision, overlap idempotency, single-item failure termination, resume behavior, key redaction, timeout, protected artist/character names, unknown special tags, no image upload by default, batch consent required on every external vision job, cancellation/no-dispatch behavior, consent audit redaction, target isolation, and reviewed-output preservation.


### Rate-limit group scope

AI API configurations maintain independent concurrency, cooldown, and adaptive-recovery state by default. Users can explicitly assign multiple configurations to a shared rate-limit group. Members of that group share the effective concurrency cap, HTTP 429 cooldown, and gradual-recovery counter. Do not assume configurations share provider-side limits based only on provider name. Configurations outside a group remain independent. Group reassignment must affect future dispatches without cancelling in-flight requests solely due to the setting change.


### Member and group concurrency caps

Each API configuration retains its own configured maximum concurrency. A shared rate-limit group also has a separately configured aggregate maximum. Every dispatch must satisfy the member-level effective limit and the group's effective limit, and the aggregate in-flight request count across group members must not exceed the group cap. Group cooldown and adaptive recovery are shared; member limits continue to constrain each member individually. Limit changes affect future dispatches without cancelling in-flight requests solely due to configuration changes.


### HTTP 429 behavior for shared rate-limit groups

An HTTP 429 from any member configuration places the entire explicitly shared rate-limit group into cooldown. No member may dispatch new requests until the cooldown expires. In-flight requests are not cancelled solely because another member received 429. After cooldown, the group resumes at active concurrency 1 and gradually recovers using the shared success counter. Honor Retry-After when present, otherwise use the configured default cooldown. Retry the request that received 429 at most once; if it fails again, defer its affected IDs to recovery. Configurations outside the group are unaffected.


### Default group concurrency maximum

A newly created shared rate-limit group defaults to an aggregate maximum concurrency of 2, regardless of the individual maxima or the number of member configurations. Users can change the group maximum explicitly. Requests must still satisfy both member-level and group-level effective limits; group creation does not silently increase aggregate concurrency.


Successful requests that were already in flight when a shared rate-limit group entered HTTP 429 cooldown may still complete and persist valid results, but do not count toward gradual concurrency recovery. When cooldown ends, the group's recovery success counter starts at zero; only fully successful batches completed after cooldown ends count toward raising the active concurrency limit.


## Authoritative rate-limit and concurrency rules

Use this section as the implementation summary when interpreting the related decision records above:

- **Isolation:** Tasks using the same saved API profile share its scheduler. Separate profiles/configurations remain independent by default, even when they name the same provider. Users can explicitly place multiple configurations into a shared rate-limit group.
- **Caps:** Each API configuration has its own maximum; a shared group has an additional aggregate maximum. Every dispatch must satisfy both applicable caps. A new shared group defaults to aggregate maximum 2.
- **HTTP 429:** A 429 sets active concurrency to 1 and starts a cooldown. In a shared group, this gates all group members. Honor `Retry-After`; absent that header, use a 5-minute cooldown. Other in-flight work is not cancelled solely due to the 429, but no new work dispatches through the affected scheduler during cooldown.
- **Recovery:** Start post-cooldown recovery at zero successful batches. Require 5 fully successful batches at active concurrency 1 before increasing to 2; require another 5 at each subsequent active level. Thresholds remain configurable and active concurrency cannot exceed the configured caps. Requests completed during cooldown may save valid results but do not count toward recovery.
- **Other errors:** Non-429 errors do not by themselves reduce concurrency or reset accumulated recovery progress. Apply the bounded, error-specific retry policy already specified above.
- **Configuration changes:** A raised configured cap does not bypass cooldown or reset adaptive state. A lowered cap constrains future dispatches. Do not cancel in-flight requests solely due to a cap or group-membership change; reconcile their accounting before dispatching more work.
- **Persistence:** Persist cooldown deadlines, active limits, recovery counters, group membership, in-flight/request reconciliation data, and resumable item outcomes. Startup must not send requests until the user explicitly resumes a saved job.

The numeric defaults above are product defaults, not provider guarantees; users may configure supported thresholds and caps. This is a user-confirmed policy specification, not a claim that the scheduler has already been implemented or tested.


## Saving and testing API configurations

An API configuration can be saved without a successful connection test. Connection testing is an explicit user action; saving alone must not send an API request. The UI distinguishes saved/unverified, test successful, and test failed or stale states. At task execution, unusable endpoints, authentication failures, and unsupported models produce actionable errors without discarding the saved configuration. Keep credentials out of logs and diagnostics.


## Pause on configuration-wide API errors

Pause the affected job immediately when a response clearly indicates a configuration-wide problem, including HTTP 401 authentication failure, HTTP 403 authorization failure, or an explicit unsupported/nonexistent model error. Persist resumable progress, cursor, queued IDs and completed outcomes; present an actionable message and require explicit user action to resume after correction. Do not keep retrying requests under the same invalid configuration or mark affected content as failed merely because of a configuration-wide error. Ordinary item-level processing errors continue under existing bounded fallback/recovery rules. Jobs on unrelated profiles may continue; configuration edits alone must not automatically resume a paused job or dispatch requests.


## Eight-pair multimodal JSON batch and bounded parallel waves

Vision optimization packs up to the profile's configured batch size (default 8) of images **and matching sanitized Danbooru raw-tag priors** in a single JSON `/chat/completions` request. It sends an `items` JSON array containing stable numeric media IDs; each image is introduced by an `image_item_id` metadata marker. JSON Schema mode binds the response item count and output type; accepted responses are still locally parsed, ID-deduplicated, and validated. The request has a hard 16 MiB body bound. A last partial batch is not padded.

The actual concurrent HTTP dispatch uses batches as units, rather than sending all eight images as separate HTTP requests. The profile's concurrency setting caps the number of simultaneous batches in a dispatch wave. HTTP calls run in scoped worker threads, never mutate SQLite, and results are correlated/persisted serially by image ID. Recovery requests remain serial and apply bounded content fallback. A concurrent 429 lowers subsequent waves to one active request, persists the cooldown, and keeps unsuccessful item IDs for recovery. Already-started requests are not cancelled merely because a peer was rate-limited.

The desktop profile setting and upload confirmation display both values separately: `batch_size` (images per JSON) and `concurrency` (parallel JSON requests). Upload consent covers the exact selected image set and model/profile, not any later job.

## Content-addressed AI result reuse

Each successfully accepted image result stores a stable SHA-256 cache fingerprint in the existing SQLite text-artifact provenance. The fingerprint covers the image SHA-256, sanitized Danbooru prior, provider URL/model, the selected system prompt, both local name formats, image upload settings and local tag-rule digest. A new explicit optimize job searches historical successful versions for that fingerprint. When `use_cache` is enabled (default), an exact hit is restored as the current `complete` sidecar without image recompression or an external request. Different prompts, images, provider models or local rules miss safely. The toggle in API profile settings can be disabled for a deliberate fresh experiment. No API credentials are included in fingerprints, and older artifacts lacking fingerprints are never silently assumed to match.

Verification note: the cache's mock HTTP fixture requires the first network request to succeed and checks that an identical second job completes without a second network call.
