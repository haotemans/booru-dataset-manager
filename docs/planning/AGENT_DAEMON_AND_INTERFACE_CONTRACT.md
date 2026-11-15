# ADR-001: Persistent Agent scheduler and CLI/MCP contract

**Status:** Partially accepted. Scheduler lifecycle **A — independent persistent service** was selected by the project owner on 2026-11-15. Other choices in this document remain proposals for discussion. No runtime changes are authorized by this document.
**PR:** #11 (draft, design-only).
**Scope:** local Windows/Linux, headless and desktop, reusable across collections. Cloud/remote transports are out of scope for the first implementation.

## 1. Accepted requirements and boundaries

1. CLI and MCP are **both designed for Agents**. GUI is for user inspection, decisions, and task control.
2. A **persistent local scheduler daemon** outlives the GUI, any single CLI command, an MCP bridge and Agent sessions; they all connect to its shared state.
3. The daemon owns local scheduling, durable progress, asset storage integration, and business-rule enforcement. It must not require a GUI to remain open.
4. The daemon does **not** presume a hosted vision/LLM API. Agent reasoning runs in the connected client's model; the server can finish local deterministic work while Agents are offline but marks model-dependent work as awaiting an Agent.
5. Existing raw tags, original media, reviewed text, consent, image hash preconditions and human confirmation of deletion remain protected.
6. Current MCP tool names stay available during migration; no silent client-breaking replacement.

**Distinction:** persistent after client close does not by itself mean start on boot. Login/boot auto-start is optional and separately configurable.

## 2. Proposed process topology

    Agent w/ shell -> bdm CLI -----------+
                                          |
    Agent w/ MCP -> bdm mcp serve --------+-> local authenticated RPC -> bdm daemon
                                          |                              |
    Human GUI -> bdm-desktop -------------+                              +-> SQLite/catalog
                                                                         +-> work graph/leases
                                                                         +-> CPU/IO/model workers
                                                                         +-> media & export storage

A suggested packaging strategy is one **headless** \`bdm\` executable with CLI, \`bdm daemon serve\`, and \`bdm mcp serve --stdio\` modes, plus the existing \`bdm-desktop\` GUI. These are separate OS processes even when they share the same binary. A separate named daemon binary is optional later; avoid forcing CLI to initialize eframe or the GUI.

- One daemon **per canonical catalog/data root** in v1 (all collections in that catalog share queues); do not start multiple daemon writers for the same catalog. Other catalog roots are isolated.
- One daemon is the **sole schedule owner**. CLI, MCP and GUI may issue concurrent requests, but never create separate independent schedulers.
- All interfaces use **versioned RPC/use-case contracts**; no CLI->MCP->CLI chaining.
- IPC proposal: Windows named pipe with per-user ACL; Linux Unix domain socket with restrictive permissions. No listening public TCP port by default. Explicitly authenticated remote access is a future design.
- RPC transport should bound request size, response size, in-flight calls and image payload. Images/large previews use scoped asset handles or controlled streaming rather than arbitrary file paths or huge JSON/base64 blobs.
- Authentication is OS identity + endpoint ACL in v1. Authorization is separate: per-client read, curation-write, job-control, export, and human-only destructive scopes. Client-provided \`model\` labels are audit metadata, **not** authentication.
- Daemon health includes protocol version, catalog identity, startup/recovery state, resource pressure and running work counts. No credentials in logs or telemetry.

## 3. Daemon lifecycle

**Lifecycle states:** stopped -> starting -> recovering -> ready <-> degraded -> draining -> stopped. Crashes and hard kills may bypass draining.

**Start:**
1. Explicit \`bdm daemon start --detach --catalog <absolute path>\` (proposed). CLI/MCP/GUI connect if already running. An optional, user-approved auto-start policy can launch it if absent.
2. Acquire an exclusive per-catalog daemon-owner lock. Check identity, process liveness and protocol; never trust a stale PID file alone or create two owners.
3. Open and validate catalog; perform transactional schema migrations with backup/checks. Recover staged image writes, interrupted filesystem/database journals and known jobs before advertising \`ready\`.
4. Reconcile prior tasks. A local deterministic stage may resume if safe. Expired Agent work leases return to \`awaiting_agent\` without applying unacknowledged model output. Awaiting-user approvals stay pending.
5. Warm only required caches and workers; defer expensive DINOv3 model initialization until needed and within configured resource budgets.

**Running:**
- Operate until an explicit stop/shutdown, controlled service-manager exit or system termination; no implicit termination just because all clients disconnect.
- GUI/CLI/MCP clients reconnect with version negotiation and incremental event cursor, no full-catalog rereads per update.
- If DINOv3 is unavailable, report degraded capability; never present pHash-only results as successful DINO inference.
- Daemon health, last error and resource pressure are visible to CLI/MCP/GUI.

**Stop / restart / upgrade:**
- \`stop --drain\` stops new leases, waits for in-flight short commits, checkpoints work, and releases owner lock. Forced stop is distinct and should be clearly reported.
- Upgrade is controlled: reject incompatible clients, stop/drain the old daemon, migrate with backup/check and start one replacement, then reconnect clients.
- Windows autostart via a user-approved login task/service option; Linux via optional \`systemd --user\`. No elevation or boot-time installation in the default path.
- Service must never silently transform \`awaiting_user\` into approval after restart.

## 4. Scheduler model (proposed)

**Hierarchy:** dataset workflow -> job -> per-media stage -> work item -> attempt/lease -> committed artifact/event.

Candidate stages (select by workflow profile, not all mandatory):

    Discover/source metadata
      -> Download & verify (optional for existing media)
      -> integrity/exact-hash/dimension gate
      -> pHash/DINO feature cache (if enabled and model available)
      -> duplicate candidate generation
      -> Agent visual inspection & reasoning (when needed)
      -> conditional crop preview/apply/reinspect
      -> annotation + quality + review result
      -> human pending review (only for ambiguous/destructive choices)
      -> export/manifest verification

Model output and local feature computation are separate stages; no LLM call for hash, dimensions, existing output checks or data movement. A rejected item does not automatically advance to deletion. Export may depend on user-specified profile/approval constraints.

**Execution states:** queued, ready, running_local, awaiting_agent, leased, awaiting_user, paused, retry_wait, completed, failed, cancelled. These are *stage/work-item* states; a job aggregates its items and uses a simpler outward state. Use an explicit transition table with atomic predicates, rather than unconstrained strings.

**Lease/commit protocol:**
1. Agent requests \`lease(job_id, max_items, supported_capabilities)\`; daemon grants bounded image IDs with revision+SHA, lease ID, monotonically increasing fencing generation, expiry, stage requirements and already-prepared evidence handles.
2. Lease is renewed by an explicit heartbeat; ordinary result submission also heartbeats. Agents are not expected to keep a socket open while reasoning.
3. Result submission supplies lease ID, fencing generation, idempotency key and expected image revision plus observations. Daemon commits each item atomically if the lease is current and image has not changed; partial success is reported per item.
4. Late or duplicated submissions are rejected or replayed idempotently. Expired work is offered again; already committed effects are never applied twice.
5. Agent disconnect leaves local stages running. Agent-required stages wait for a new Agent. Do not claim exactly-once external model execution; provide at-least-once attempts with idempotent stored effects.

**Per-image consistency:** multiple Agents may assess different images, but the same image revision may have only one active mutation owner. An active GUI edit invalidates stale leases. Shared raw/original artifacts are immutable. Crops change the current revision and invalidate image-dependent captions/scores according to explicit rules.

**Critical sections:** hold DB/file locks only to validate+commit+publish durable state. Image decoding, DINO inference, network, crop-preview construction and Agent model thinking execute without the global catalog/media write lock. Use atomic filesystem journals with startup reconciliation and short coordinated commit.

**Resource pools & fairness:** independently bounded resources for provider network/API, download IO, image decode, hashing, DINO inference, Agent leases, database writes and export. Backpressure limits resident images and total in-flight bytes. Interactive/GUI work has higher responsiveness while bulk jobs cannot be starved. Use configurable budgets and weighted scheduling; determine numerical defaults from benchmarks. Do not assume the daemon can regulate remote Agent token rate unless the Agent exposes capacity information.

**Caching and skip logic:** immutable exact-source hash; derived asset cache keyed by image revision + operation/model version + settings/profile fingerprint. Reuse pHash, DINO embeddings, thumbnails, contact sheets and completed valid captions. Edited-image revisions must invalidate only dependent artifacts; no cache is evidence of a prior full image inspection by a new Agent.

**Persistent storage (illustrative, not migration-ready):** scheduler_jobs, scheduler_items, scheduler_attempts_or_leases, scheduler_events, idempotency_keys and asset_cache. Catalog retains authoritative media provenance. Persist event sequence for incremental GUI/Agent polling; indexes for job/status/due-time/collection/media revision; benchmark SQL pagination rather than loading whole tables.

## 5. Interface design: one core, different ergonomics

Every public capability specifies: input/filter, expected revision, actor/permission, output schema, skip/error codes, batch/stream limits, and idempotency. CLI and MCP share domain services, not a requirement for the same low-level transport shape.

| Operation | Agent CLI proposal | MCP proposal | Output, scheduling or safety requirement |
| --- | --- | --- | --- |
| Capabilities | \`bdm capabilities --json\` | \`bdm_capabilities\` | version, permission scopes, limits, supported tools, resource availability |
| Daemon status/control | \`bdm daemon status/start/stop\` | \`bdm_service_status\` read-only | only authenticated owner controls shutdown; MCP does not expose daemon shutdown by default |
| Collections | \`bdm collections list --json\` | existing \`bdm_collections\` | indexed type/provider/tag filters, opaque cursor, count |
| Media query | \`bdm media list/inspect --json\` | existing \`bdm_list_media\` | field projection, stable sort, current SHA, compact response, pagination |
| Image presentation | \`bdm media view --output <file>\` | existing \`bdm_view_image\` plus proposed \`bdm_view_batch\` | scoped asset handle or image blocks; thumbnails/contact sheets only for triage; full current view before mutation |
| Job submit | \`bdm jobs submit --input job.json\` | \`bdm_job_submit\` | durable job ID, workflow profile/version, dedupe key, dependency plan |
| Job inspect/events | \`bdm jobs status/events --json[l]\` | \`bdm_job_status\` | aggregates, per-stage errors, incremental event cursor, no unbounded log |
| Job pause/resume/cancel | \`bdm jobs pause/resume/cancel\` | \`bdm_job_control\` | scope-controlled, checkpoint and safe cancellation semantics |
| Agent work lease | \`bdm work lease --job ... --json\` | \`bdm_work_lease\` | bounded batch of ID/revisions; expiry/generation; no duplicate active lease |
| Lease renewal | \`bdm work heartbeat --lease ...\` | \`bdm_work_heartbeat\` (or bundled renewal) | explicit expiry, reject lost ownership; low overhead |
| Agent result commit | \`bdm work submit --input @-\` | \`bdm_work_submit\` | score+caption+tags+text regions in one result if no crop; item-atomic and idempotent |
| Crop/restore | \`bdm media crop preview/apply\` / \`restore\` | existing \`bdm_crop\`, \`bdm_restore_original\` | exact preview binding, 32 alignment, full verified original, revision compare |
| Duplicate candidates/decision | \`bdm dedup scan/pairs/resolve\` | existing \`bdm_scan_duplicates\`, \`bdm_resolve_duplicates\` | asynchronously scan local features; visual comparison of both; no automatic deletion |
| Review queue | \`bdm review pending/decision\` | read-only review listing; scoped review updates | human override, stale decisions and confirmation gate |
| Export | \`bdm export submit/status\` | \`bdm_job_submit\` with \`export\` kind | manifests, output conflict checks, resumability and profile controls |

**No new command or MCP tool above is implemented yet.** Keep legacy ten MCP tools operating as compatible adapters during rollout. Explicit v2 capability negotiation or a distinct server name can avoid injecting all old+new descriptions into every Agent prompt; exact migration strategy remains open.

CLI v1 requirements:
- Headless, non-interactive by default. Machine output on stdout only (default JSON); \`--jsonl\` for stream/event feeds; logs on stderr.
- Stable versioned envelope with \`request_id\`, \`ok\`, \`data\`, \`error.code\`, \`retryable\`, \`current_revision\`, and \`next_cursor\` where applicable.
- Nonzero exit codes for errors; clear partial-batch result and differentiated unavailable/conflict/permission/retryable outcomes.
- Accept \`--input @-\` for piped JSON/JSONL; avoid passing huge captions in shell arguments. Token-aware compact default fields and a \`--fields\` selector.
- CLI connects to the existing daemon; it must not spin up a second scheduler, nor require a GUI or model. Explicit start is supported; automatic startup remains policy-controlled.

MCP v1/v2 requirements:
- Expose actions as discoverable, schema-bounded tools with short descriptions and precise field constraints; keep image pixels in MCP image blocks, not model-facing base64 JSON.
- Provide an optional bounded multi-image triage tool with stable \`id -> frame\` mapping. Images selected for mutation still require full-current-image inspection; contact sheets alone do not satisfy it.
- Keep read, curation-write, and human-destructive capabilities separate; do not trust \`model\` or \`reason\` text as permissions.
- Prefer bulk result submission for multi-output work; do not introduce an extra model call merely to run separate score/tag/caption mutations.
- Bridge can exit or reconnect without cancelling daemon jobs or work already committed.

## 6. Example Agent flow (illustrative, not implemented)

    bdm capabilities --json
    bdm daemon status --json
    bdm jobs submit --input curate-job.json
    bdm jobs status JOB_ID --json
    bdm work lease --job JOB_ID --max-items 8 --json
    # Agent inspects provided image blocks/assets and reasons in its own model.
    # Agent previews crop only when justified, then verifies post-crop revision.
    bdm work submit --input @-               # typed JSON batch on stdin
    bdm jobs events JOB_ID --after EVENT_ID --jsonl

A **single visual reasoning pass** may produce score, evidence, caption, visual tags, text regions, and proposed crop. For uncropped media, commit these together. For cropped media, preview -> apply -> re-view final image -> commit final-image-consistent annotation. Never write an annotation based on an earlier crop revision without validation.

## 7. Performance methodology and risks

Measure before refactoring against fixed synthetic and explicitly approved production-like datasets:

- Tool/CLI round trips and model image-token budget per accepted item; count duplicated image presentations.
- Stage throughput (images/min), p50/p95 latency, local worker utilization, time awaiting Agent, batch retry rate.
- SQL reads and scanned rows, cache hit/miss, lock hold p95, image decode/encode, RAM peak and disk pressure.
- Validation guardrails: exact identity/tag preservation, SHA and original integrity, race/cancel/retry correctness, user-review retention, zero unintended destructive action.

Known code-review candidates, not verified bottleneck measurements:
- \`AgentServer.task\` holds a \`Mutex<SqliteCatalog>\` for each tool task (agent/mod.rs).
- \`tools::collections\` / \`tools::list\` load broad catalog/media/artifact sets before filtering (agent/tools.rs).
- \`tools::scan\` holds shared \`media_lock\` during visual analysis (agent/tools.rs).

Optimization priority: instrument and baseline -> SQL projection/keyset paging -> staged workers and brief commit locks -> Agent batch presentation + combined results -> cache policy -> multi-Agent lease tests. Never claim numerical speedups before measuring.

## 8. Implementation and verification sequence

1. **Contract + baseline**: freeze version/error model, security scope, operation stages, and a reliable measurement harness. No data migration yet.
2. **Daemon skeleton**: single-instance lock, IPC, health, recovery, startup/stop and reconnect; tests on Windows and Linux.
3. **Durable scheduler**: job/item/event/lease storage and migration backup; crash recovery, lease fencing, fair scheduling and resource budgets.
4. **Headless CLI**: capability/query/job/work/image commands over RPC; contract and safe stdout/exit-code tests.
5. **MCP bridge**: preserve existing ten commands while routing through use cases/daemon; add batching where measured. Regression-test Antigravity client integration.
6. **GUI coordination**: incremental event subscription, user priority/override, reviewer confirmation and clean restart handling.
7. **Benchmark & release**: fixed datasets, concurrency + fault injection, integrity and security tests, packaging; publish measured before/after results.

No code should be merged solely because unit tests pass: crash recovery, RPC reconnect, stale leases, concurrent GUI edits, and production-format image fixture tests must pass.

## 9. Open design decisions for next discussion

1. **Agent execution ownership:** Agent clients pull/heartbeat work leases (recommended for API-neutral CLI+MCP), versus a daemon-driven adapter that launches Agent sessions. The daemon does not embed a provider-specific model by default.
2. **Daemon start policy:** explicit start by default, or consented lazy-start when CLI/MCP/GUI cannot connect.
3. **Work unit granularity:** individual-image commit with dynamically sized batch leases (recommended), versus fixed batch atomicity.
4. **MCP exposed-tool budget:** v1 compatibility and v2 capability negotiation/visible tools; decide the minimal low-token surface.
5. **Queue budgets/benchmark thresholds:** set after profiling; default to safe bounded concurrency, never promise static throughput.
6. **User review override precedence:** precise transitions for human edits while an Agent lease is outstanding.

Return to the parent RFC for overall scope: [AGENT_FIRST_CLI_MCP_SCHEDULER_RFC.md](AGENT_FIRST_CLI_MCP_SCHEDULER_RFC.md).
