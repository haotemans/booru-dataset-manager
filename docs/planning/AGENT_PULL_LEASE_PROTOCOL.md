# ADR-002: Agent-pull work leases, batching, and resource-aware scheduling

**Status:** Partially accepted. Owner chose **Agent-pull (option A)** on 2026-11-15, on top of **ADR-001 persistent daemon (option A)**. The wire shapes, defaults, concrete interface names, and resource policies below are **proposals pending review**.
**PR:** #11, draft; design only; no runtime/API/migration changes.
**Companion:** [ADR-001](AGENT_DAEMON_AND_INTERFACE_CONTRACT.md); [parent RFC](AGENT_FIRST_CLI_MCP_SCHEDULER_RFC.md).

## 1. Accepted decision and immediate consequences

- Agent **actively claims** work via CLI or MCP; the daemon **does not spawn or call** an arbitrary hosted/client Agent or select the client model.
- The independent daemon persists the stage graph, ready queues, leases, file and metadata state. Local deterministic jobs continue when Agent clients disconnect; model-reasoning stages remain `awaiting_agent` until an Agent claims them.
- No permanent Agent registration, callback server, inbound Agent port, or provider-specific model API is required for v1.
- The same job may be continued by a different Agent after expiry or explicit release. The new Agent must retrieve current evidence and revision; previous uncommitted reasoning cannot be presumed valid.
- CLI and MCP have the same semantics through daemon RPC, while their binary/image representation differs. The GUI stays available for human override and final destructive confirmation.

**Rejected for v1:** daemon push/callback scheduling as the primary execution model; automatic Agent launches as a requirement; one process-local queue per interface.

## 2. Work graph and ownership model (proposed)

Hierarchy: `workflow -> job -> stage -> work item -> lease attempt -> durable result/event`.

Proposed stage kinds:
1. `discover`, `fetch`, `integrity`, `exact_dedup` — local IO/network, no Agent lease.
2. `feature_extract`, `similarity_candidates` — local pHash/DINO, no Agent lease; mark capability unavailable explicitly.
3. `visual_curation` — Agent lease for classification, caption, visual tags, text regions, and crop recommendation in one reasoning pass where justified.
4. `crop_review` — conditional Agent-controlled preview/apply; final current-image reinspection required.
5. `pair_review` — Agent lease references **both image IDs and both SHA-256 revisions**. Must have both visually inspected; never infer duplicate from character identity alone.
6. `human_approval` — no Agent lease for final deletion. Human review can veto/cancel any pending rejection.
7. `export` — local job, durable and manifest-checked; no Agent reasoning unless a new explicit stage requires it.

**Primary work-unit granularity proposal:** one media item and one input revision per `visual_curation` item; acquire multiple independent items in one **batch lease** to cut round trips. Each result commits independently, not as an all-or-nothing batch. `pair_review` is a distinct two-image work item with atomic, sorted acquisition of both media-scoped mutation claims (avoid deadlock).

**Cross-job dedupe:** overlapping workflows must not double-schedule equivalent per-image/stage work. Reuse completed artifacts when stage input SHA, profile fingerprint, policy version, tool/model version, and validation conditions match. An active item can have multiple dependent jobs but only one effective processing owner; different outputs or policies must not be incorrectly coalesced. Preserve human decisions.

## 3. Lease lifecycle and state transitions (proposed)

Agent-visible work states: `awaiting_agent -> ready -> leased -> completed`, with permitted exits to `released` (returns ready), `retry_wait`, `awaiting_user`, `failed` and `cancelled`. A paused job is ineligible for a new lease.

**Claim algorithm:**
1. Verify authenticated client and scoped permissions, declared capabilities, image/provider consent, job state, worker budget and protocol version.
2. Pick eligible ready work using fair scheduling, priority, oldest-ready-time, and per-client limits.
3. Within one short database transaction, fence previous ownership, atomically claim each needed media-scoped mutation claim, write `lease_id`, monotonic `generation`, `expires_at`, `owner_id`, and attempts.
4. Return a bounded list of items with expected SHA/revision, stage, identity-protected metadata, minimal evidence manifest, requirements, deadlines and a resume cursor. No image base64 in generic work-list JSON.

**During execution:** Agent calls view/asset tools, optional detail tiles, crop preview/apply, and keeps the lease fresh with heartbeat or other valid item calls. A short-lived CLI invocation does not keep the lease alive by itself; the Agent loop must renew before expiry. Agent can submit individual items before a batch finishes. An item accepted as completed is removed from the lease and cannot be resubmitted with different effects.

**Submit:** each item includes `work_id`, `lease_id`, `generation`, `expected_sha256`/revision, `idempotency_key` and typed results. Daemon revalidates ownership, lease freshness, permissions, inspected-image evidence, human override and current media revision; commits each item atomically with its audit event, or returns a precise per-item error. Same key + same request fingerprint returns stored result; same key + different fingerprint is rejected.

**Expiry/disconnect:** a lease may outlive a disconnected bridge/CLI process until TTL. After expiry, a new claim increments the fencing generation; all late older-generation writes are denied. Agent reasoning may be repeated, but durable effects remain idempotent. Explicit `release` returns unfinished items quickly; do not wait for TTL where a healthy Agent can release.

**Pause/cancel:** stop new claims immediately. Running work can finish only an explicitly permitted safe commit, otherwise the daemon fences it. Cancellation never rolls back already committed non-destructive evidence, never erases immutable source data, and never silently approves pending deletion.

**Suggested *starting* timeouts for benchmarking, not final defaults:** 10-minute lease TTL, heartbeat by roughly one-third of remaining TTL, bounded maximum renewal window with fairness controls. Adapt to visual model latency or user settings, and test long-running reasoning and CLI interruptions. Persist UTC expiry plus owner generation; use a monotonic clock for live waiting where possible, reconcile wall-clock changes on recovery.

## 4. Versioned lease request/response (illustrative; names and exact schema are not implemented)

### Claim work

CLI: `bdm work lease --job JOB_ID --max-items 8 --capabilities visual,caption,crop --json`

MCP: `bdm_work_lease` with equivalent validated arguments.

~~~json
{
  "api_version": "v2-draft",
  "job_id": "job-72",
  "max_items": 8,
  "capabilities": ["visual", "caption", "tags", "crop"],
  "wait_ms": 0,
  "agent_instance_id": "agent-session-4"
}
~~~

Response (abbreviated):
~~~json
{
  "lease_id": "opaque-lease",
  "generation": 12,
  "expires_at": "2026-11-15T10:00:00Z",
  "items": [
    {
      "work_id": "work-123",
      "media_id": 123,
      "stage": "visual_curation",
      "expected_sha256": "64-hex-current-image-digest",
      "revision": 7,
      "required_evidence": ["full_current_image"],
      "image_ref": "scoped-asset-handle",
      "current_annotation_state": "needs_review",
      "protected_identity_source": "source_tags"
    }
  ],
  "remaining_ready_hint": 28,
  "retry_after_ms": null
}
~~~

`agent_instance_id` is diagnostic metadata bound to an authenticated caller; it is **not** authentication. Lease IDs/asset handles are bearer capabilities and must never be printed in unrestricted logs, shared with another Agent, or exposed via arbitrary file paths. Exact file/model names must not be injected into model context unless necessary.

### Image access / inspection

- CLI: `bdm work view --lease LEASE --item work-123 --kind current --output <scoped path>`; prints metadata/receipt, writes pixels only to the requested safe local file or controlled stream. For headless Agent with read access, the client explicitly loads the returned image.
- MCP: `bdm_work_view` returns a compact metadata block **and** MCP image content block(s), not JSON/base64 text.
- A `full_current_image` receipt is tied to (authenticated client, lease, item, media ID, revision, full-frame view). Detail crops or contact sheets alone do **not** issue that receipt.
- An exact crop-preview receipt binds the proposed bbox, source revision, output geometry and preview fingerprint. Applying a crop requires that receipt plus fresh source precondition; after apply, new revision and image receipt are required before final-image annotation.
- This proves the service **served** the image/preview under the correct lease; it does **not** prove a model genuinely interpreted its pixels. Never claim cryptographic proof of visual understanding.
- A visually useful 1024px display can constitute a complete-frame view for inspection; retain native pixel dimensions for editing and provide tiles when necessary for small text. Do not confuse display resolution with saved edit resolution.
- External Agent image transmission is controlled by explicit source/consent policy; daemon does not independently upload images to a model provider.

### Submit partial or completed results

CLI: `bdm work submit --input @- --json` (JSON stdin; do not place large captions or lease secrets directly in process arguments).

MCP: `bdm_work_submit` (bounded item array; same use-case contract).

~~~json
{
  "api_version": "v2-draft",
  "lease_id": "opaque-lease",
  "generation": 12,
  "items": [
    {
      "work_id": "work-123",
      "expected_sha256": "64-hex-current-image-digest",
      "idempotency_key": "agent-session-4:work-123:revision-7:commit-1",
      "decision": "keep",
      "quality": {"aesthetic": 8.5, "training_suitability": 9.0},
      "caption": "Visual description based on the current image.",
      "tags": ["1girl", "red eyes"],
      "text_regions": [],
      "crop_proposal": null,
      "reason": "Distinct complete composition"
    }
  ]
}
~~~

Response has `accepted`/`skipped`/`rejected` arrays, each entry including `work_id`, `status`, `current_revision`, and, on error, stable `error.code` and `retryable`. Partial batch failure is normal and must not be represented as complete global success.

**Result transaction boundaries:** shared lease/scope/image-revision/human-edit preconditions fence the item before any new effect. Independent selected output targets then commit as separately acknowledged journaled units; a rejected generic tag does not roll back a valid caption or assessment. An all-target atomic group requires explicit plan semantics. Preserve manual content unless overwrite is explicitly authorized. [ADR-008 section 6](AGENT_ANNOTATION_REGION_CONTRACT.md#6-reconcile-per-item-fencing-with-partial-output-success) defines per-target revisions, durable outcome lookup and correction keys. Implementation requires a journal across SQLite and sidecars before claiming recovery guarantees. For a crop proposal, the proposal alone is not an instruction to modify; a separate explicit exact-preview/apply path produces a **new revision**. The old annotation result must not be committed onto the new image without its own verified post-crop evidence.

### Heartbeat / release

CLI: `bdm work heartbeat --input @- --json`, `bdm work release --input @- --json`.

MCP: `bdm_work_heartbeat`, `bdm_work_release` (or folded into one lease-control tool if exposure costs are too high).

Heartbeat renews only unfinished, still-owned items. A stale generation gets `LEASE_LOST`, not a revival. Release accepts a reason (capability mismatch, cannot inspect image, retry later) and preserves failure history. Server may attach a new expiry after a valid view/submit request to save calls.

## 5. Batching: throughput without blowing up Agent context

| Resource/operation | Initial benchmark *candidate* | Why / boundary |
| --- | --- | --- |
| Agent work lease | 4-8 media IDs (max 16 before profiling) | One cheap metadata result; independent item commits |
| Image content returned to MCP model | 1-4 images per tool result | Explicit bounded bytes/pixel budget; no forced base64 text |
| Contact sheet triage | 4-16 labeled thumbnails, configurable | Fast candidate selection; insufficient alone for mutation |
| Agent result submit | up to 8-16 typed item results | Partial acknowledgment, individual idempotency |
| Local pHash / DINO request | 16-64 metadata IDs | Feature cache and local pool, not a blocking MCP call |
| Download metadata/page | 32-128 rows with SQL projection | Keyset pagination, max response byte budget |
| SQLite write | short single item/journal commit | Never lock globally across inference or Agent reasoning |

These are *hypotheses*, not verified optimal numbers. Daemon advertises effective limits in `bdm_capabilities`. Client can request less. Adaptive batch control should shrink on model context pressure, image size, high error/retry rates, or slow response and enlarge only when measured improvements hold. Keep independent controls for number of items, number of pixel images, encoded byte budget, and Agent token budget. A contact sheet can save calls, but meaningful large artwork details must remain accessible.

**Avoid two common inefficiencies:** (1) returning raw images alongside every list/lease call, (2) rescanning the same content after every submission. Use IDs, revision-aware asset handles, a compact manifest, and delta/event cursors.

## 6. Scheduler resource management

- Resource classes: source API/network, image download bytes, CPU decode/hash, DINO/model inference (CPU currently), temporary image buffers, DB writer, export disk IO, and remote Agent work slots.
- Daemon controls local worker slots, memory and queue depth. For externally hosted Agent inference, the daemon controls only **work leases** and reported client capacity, not actual provider token consumption or API parallelism.
- Enforce per-job and per-Agent lease caps, per-resource semaphores, bounded input queues, and memory/byte limits. Use weighted-fair scheduling to keep an interactive request responsive without starving bulk jobs.
- Prefer stage-level pipelines: while an Agent handles batch N, deterministic workers may preprocess N+1, subject to backpressure. Do not proactively decode gigabytes merely because a queue is deep.
- Apply `single-flight` to expensive derived work keyed by (media revision, model/version, params). In-progress feature work is shared across dependent jobs; a model failure leaves a distinct incomplete state, not false success.
- Use SQLite WAL (subject to Windows/Linux testing), indexed keyset reads and short serialized writer transactions. Distinguish internal DB writer serialization from long exclusive media/file locks; never hold global locks across network/model/image codec operations.
- Persist progress/event sequence. Clients use `jobs events --after SEQUENCE` or MCP status/event cursor; CLI process exit, MCP bridge restart and GUI redraw must not trigger full-catalog re-query.
- Respect active human edits: if GUI changes an image, the daemon increments revision and fences all stale dependent Agent mutations immediately. Human reject cancel/keep override cannot be silently reversed by an older Agent attempt.

**Cross-media pair decision:** the two-image `pair_review` lock reservation must be acquired atomically in sorted ID order with a brief transaction. If one image changed, reject the pair result; never transfer collection associations or queue deletion based on stale survivors.

## 7. CLI/MCP operation mapping (proposed)

| Agent goal | CLI | MCP | Key requirements |
| --- | --- | --- | --- |
| Learn available abilities | `bdm capabilities --json` | `bdm_capabilities` | server version, limits, scoped permissions |
| Submit workflow | `bdm jobs submit --input FILE` | `bdm_job_submit` | idempotent, durable job ID and plan |
| Check queue/work | `bdm jobs status/events` | `bdm_job_status` | compact deltas; failures and awaiting-Agent counts |
| Claim batch | `bdm work lease --job ID --max-items N` | `bdm_work_lease` | same ownership/fencing semantics |
| View full current image | `bdm work view --lease ...` | `bdm_work_view` | full-frame inspection receipt; content blocks for MCP |
| View labeled triage | `bdm work contact-sheet ...` | `bdm_work_contact_sheet` | stable media ID/frame mapping; not mutation evidence |
| Extend lease | `bdm work heartbeat --input @-` | `bdm_work_heartbeat` | reject lost leases; optional bundled renewal |
| Submit analyses | `bdm work submit --input @-` | `bdm_work_submit` | single pass multi-output; per-item atomic |
| Crop if needed | `bdm media crop preview/apply` | existing `bdm_crop` | bound preview & new revision, reinspection |
| Yield / give up | `bdm work release --input @-` | `bdm_work_release` | fast reassignment with reason |
| Agent diagnostics | `bdm jobs events/stats` | `bdm_job_status` | no log flooding; failure categories |
| Pausing workflow | `bdm jobs pause/resume/cancel` | `bdm_job_control` | scope-controlled, durable state |
| Final deletion approval | human GUI | NOT available to Agent by default | explicit human consent, never implied |

New names are illustrative. Keep the ten existing MCP tools accessible via compatibility adapters; version-specific capability advertisement must avoid registering dozens of unused tool descriptions for every Agent.

## 8. Error contract and retry matrix (proposed)

| Code | Retry? | Agent response |
| --- | --- | --- |
| `NO_READY_WORK` | after server hint | Do not busy-poll; wait or switch job |
| `LEASE_LOST` / `LEASE_EXPIRED` | re-claim + inspect | Discard stale result; no blind resubmit |
| `REVISION_CONFLICT` | re-view + re-analyze | Human edit or current image changed |
| `INSPECTION_REQUIRED` | yes, after full view | Contact sheet is insufficient |
| `CROP_PREVIEW_REQUIRED` | yes, after exact preview | Do not commit unpreviewed crop |
| `PERMISSION_DENIED` | no, until scope changes | Do not escalate automatically |
| `MODEL_UNAVAILABLE` | later/manual fallback | Do not call failed pHash-only DINO scan successful |
| `RATE_LIMITED` / `RESOURCE_BUSY` | yes, after delay | honor retry_after_ms with jitter |
| `DATA_INTEGRITY` | human intervention | quarantine; do not delete source |
| `IDEMPOTENCY_CONFLICT` | no | same key used with different payload |
| `REVIEW_REQUIRED` | human | never convert to deletion |

Query the durable original outcome after an ambiguous response before doing more model work. An authorized identical replay of a committed request returns its stored acknowledgement without new effects, even if its old lease has since expired; an uncommitted new write still requires current ownership/evidence. Corrections use new keys, select failed targets only and preserve already committed outputs. Never retry non-idempotent side effects on an ambiguous response without first querying outcome by idempotency key. Bound retries, persist attempts, and use backoff with jitter. Record failed/skipped/completed IDs and reasons so Agents do not start again from the beginning.

## 9. Recovery and adversarial tests required before implementation completion

1. Two Agents simultaneously leasing the same media; only one may own its mutating stage.
2. Overlapping jobs share expensive feature computation but not mismatched profiles/policies.
3. Lease expiry and delayed submission with old fencing generation: no stale write.
4. CLI exits mid-batch; daemon continues local stages and reoffers expired Agent work.
5. MCP bridge restarts mid-view/crop; client may reattach safely, no lost recorded results.
6. GUI crop or human-review override racing an Agent result: reject stale Agent write.
7. Mixed batch submit: valid items commit once, invalid ones return typed errors; retry cannot double-apply.
8. Crash between staged PNG/TXT write and SQLite commit: journal reconciliation preserves originals and approved text.
9. Pair-review result with one changed media or rejected survivor: no unintended association transfer/deletion.
10. Extreme image size, multiple 2048+ inputs and slow model: bounded resident memory and response bytes; Agent context limit respected.
11. Multiple daemon launches for one catalog: exactly one scheduler owner; second connects/fails safely.
12. Network/DINO unavailable: degraded and retry states are honest; no spurious success/deletion.

## 10. Discussion decisions still open

- **Work-item granularity:** recommend per-image atomic results with dynamically sized multi-item leases, plus a separate two-image pair-review unit. Not yet owner-approved.
- **How to budget images vs context:** recommend 4-8 metadata items and up to 1-4 full-frame images per presentation as initial benchmark candidates; adapt dynamically.
- **Agent consent and model image policy:** user authorization for transmitting image content to client-selected model and logging limits.
- **Resource policy defaults:** fair scheduling and bounded pools are design goals; numerical limits require baseline measurements.
- **Tool exposure:** minimal MCP tool groups and version negotiation vs a long flat registry.
- **Start policy:** daemon remains persistent but whether CLI/MCP may lazy-start it is undecided.

Do not merge this RFC or start implementation until the proposed wire contract and resource policy receive review.
