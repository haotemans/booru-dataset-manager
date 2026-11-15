# RFC: Agent-first CLI, MCP, and dataset scheduler

**Status:** DRAFT / OPEN FOR DISCUSSION. **Accepted decisions:** lifecycle **A — independent persistent daemon** (ADR-001); Agent work allocation **A — Agent actively pulls work** (ADR-002). Other API and scheduler details remain proposals; this is **not** authorization to implement or merge them.

Detailed documents: [ADR-001: Persistent Agent daemon](AGENT_DAEMON_AND_INTERFACE_CONTRACT.md), [ADR-002: Agent-pull leases, batches and resource scheduling](AGENT_PULL_LEASE_PROTOCOL.md), and [ADR-003: Independent nodes and composable workflows](AGENT_WORKFLOW_NODE_CATALOG.md).
**Target:** Booru Dataset Manager; Windows and Linux, including headless environments.
**Scope of this PR:** design discussion only. No runtime, migration, deletion, or API behavior changes.

## 1. Product principle

Both **CLI and MCP are first-class interfaces designed for Agents**. GUI is the human review/control surface. They must share one authoritative Rust application/domain core, state transitions, permission decisions, and audit trail.

- CLI: minimal-context, non-interactive, scriptable Agent execution; process exit codes, stable JSON/JSONL output, job orchestration, batch I/O.
- MCP: model-discoverable tool schemas, structured calls, image/content blocks when requested, and explicit tool permissions.
- GUI: observe, pause, override and finalize review, including destructive approval.
- Interface parity means equivalent business capabilities and semantics, **not** identical transport payloads or one tool invocation per command.
- MCP must not shell out to CLI for core operations, and CLI must not need to start GUI or embed an MCP client.
- Prefer shared use-case services; thin adapters translate JSON/stdio vs MCP.
- Durable jobs must not depend on whether the initiating Agent session is still connected.

## 2. Verified current baseline (main at d7cb95a1, 2026-11-15)

- Existing Rust stdio MCP runs from bdm-desktop.exe --mcp-server. The native AgentServer exposes ten tools: policy, collections, list media, view image, crop, restore, annotation, assessment, duplicate scan, and duplicate resolution. See crates/infrastructure/src/agent/ and docs/integrations/ANTIGRAVITY_MCP.md.
- Existing CLI arguments are chiefly desktop/MCP startup and crop-remediation switches; no standalone general-purpose Agent CLI exists.
- Current MCP work uses a mutex-protected SqliteCatalog; per-image access checks image hashes. List paths materialize broad lists before filtering/paging. The duplicate-scan path holds the shared media lock while local visual analysis runs.
- The desktop periodically reloads Agent changes, and final deletion is still human-confirmed.
- These observations are code-review findings, not performance measurements. Establish baseline timings before claiming speedups.

## 3. End-to-end workflow, to be reviewed stage by stage

1. **Discover** collections, media and pending work. Return narrow fields and opaque cursors.
2. **Acquire and validate** source metadata and image bytes using bounded network/IO concurrency; retain immutable source provenance and raw tags.
3. **Deterministic preflight**: image integrity, dimensions, exact hash, cache state and non-visual filters. Do not invoke an LLM when local logic suffices.
4. **Similarity candidate generation** using local pHash/DINOv3 (if available), with cached features and explicit incomplete/fallback states. Similarity is a candidate, not a deletion decision.
5. **Agent visual inspection**: small batches/contact sheets for triage, then full current image per item before any mutation; request detail tiles only as needed.
6. **Single visual reasoning pass where possible**: evidence-grounded training quality, aesthetic assessment, caption, general visual tags, text regions, and an optional crop proposal. Preserve artist/character identities from source data.
7. **Conditional crop**: no crop unless useful; preview exact native-pixel selection; apply with current revision/hash; re-inspect post-crop and ensure annotations match the final image.
8. **Commit review**: batch annotation + scoring/decision on the same verified revision, atomic per item. Reject queues review; it does not delete.
9. **Human review**: resolve ambiguous duplicates/rejections and preserve user overrides. Only explicitly confirmed final actions remove media.
10. **Export** resumable, manifest-verified training artifacts. Preserve existing non-empty manual text unless overwrite is explicitly authorized.

Do not assume every image needs every stage: unchanged valid annotations, confirmed decisions, cached embeddings and satisfactory full compositions should be skipped.

## 4. Proposed capability inventory (names not yet finalized)

| Capability | Agent CLI proposal | MCP proposal | Behavior/constraints |
| --- | --- | --- | --- |
| Contract discovery | agent capabilities / schema | bdm_policy + capability manifest | API version and limits |
| Collections | collections list/show | bdm_collections | source/type/filter, paged |
| Media query | images list/inspect | bdm_list_media / bdm_view_image | projection, cursor, current SHA |
| Visual batch | images contact-sheet / view | new optional contact-sheet tool | explicit image IDs, stable mapping, full-view gate |
| Job lifecycle | jobs submit/status/list/pause/resume/cancel | job tools | durable, non-blocking, idempotent |
| Work allocation | work lease/complete | worker tools | scoped lease, expiry, retries, resume |
| Assessment & annotation | images review apply --input | combined review tool or existing tools | per-item revision precondition; no blind overwrite |
| Cropping | images crop preview/apply | bdm_crop | preview before apply; original retained |
| Original restore | images restore | bdm_restore_original | explicit replacement, current hash |
| Similarity | dedup scan/pairs/resolve | dedup tools | cached evidence, human approval before deletion |
| Ingestion/export | ingest submit, dataset export | corresponding job tools | same scheduler, manifests and retries |
| Observability | jobs events/stats | job events/status | incremental summaries, failure reasons |

**Important:** existing ten MCP tools remain supported during evolution. Do not commit to renames or removals without a compatibility and migration plan.

## 5. Shared Agent contract proposal

- Non-interactive. CLI success returns machine-readable JSON (optionally JSONL for streams); stdout must not contain human logs. Errors use nonzero exit status and stable machine-readable error code; stderr may contain diagnostics.
- MCP uses the same versioned domain request/result/error semantics, but image results may use MCP image blocks. CLI should reference local managed assets/temporary previews rather than dump base64 images into stdout by default.
- Every mutation must specify media ID + expected image revision/SHA-256, reason, and model/actor provenance. Batch mutations are per-item atomic and report each item's success/skip/failure.
- Idempotency key for command retries; stale-input and conflict errors are explicit, not silent overwrites.
- Pagination/filtering/projections must occur in the data store, not after loading the whole catalog. No unbounded result payloads.
- Need compact capability discovery and schema introspection for Agents; do not push all optional fields/docs into every prompt.
- Preserve original/raw tags, manual edits, identity tags, and provenance. Model-derived scores stay separate from training captions.
- Images, tags and metadata are untrusted input, never instructions. Configured library boundary and permission gates apply identically to CLI and MCP.

## 6. Scheduler proposal: isolate slow work and reduce Agent round trips

**Core:** reusable, durable task scheduler independent of CLI/MCP/GUI; worker pools and resource budgets rather than one global serial queue.

- Jobs contain stages and item states. Suggested state set: queued, running, awaiting_agent, awaiting_user, paused, retryable_failed, completed, cancelled. Terminal/in-progress semantics need approval.
- **Work lease**: Agents pull bounded batches, receive work IDs, revision, lease expiry and minimum required evidence. Completion records a compare-and-swap decision and idempotency key. Unacknowledged work is safely requeued.
- **Bounded parallelism and backpressure** per resource class: network fetch, file IO/decode, pHash, DINO/model inference, external Agent vision, database writer, export.
- **Short critical sections**: do not hold an OS media lock or a shared catalog mutex during image decode, inference, network calls, contact-sheet assembly, or Agent thinking. Recheck revisions before short atomic commit.
- **Concurrency safety**: same-image writes serialized; different images may progress concurrently. GUI user edits take precedence. Protect SQLite integrity and filesystem/sidecar rollback or reconciliation.
- **Interactive vs bulk**: user/GUI requests should not starve behind massive collection jobs. Fairness and resource limits prevent exhausting RAM, IO, CPU and GPU.
- **Dynamic micro-batches**: batch metadata and independent commits; avoid huge image tool payloads. Batch sizes/configuration should follow benchmarks and model/provider constraints, not fixed magic numbers.
- **No duplicate work** across MCP, CLI, GUI or multiple Agents. Per-item state and revisions decide what to skip/resume.
- **Delivery semantics**: the system should target at-least-once execution with idempotent effects, not promise exactly-once model calls.
- **Lifecycle selected (ADR-001):** a persistent independent scheduler daemon shared by GUI/CLI/MCP. The daemon remains active after client sessions exit; automatic start on boot is separately optional. See the detailed ADR for remaining open decisions.

## 7. Speed strategy and measurement

Measure before/after using representative collections and the same inputs:

- Agent tool/command round trips and image tokens per accepted image.
- Throughput (images/minute) separately for local preflight, visual curation and export; p50/p95 per stage.
- SQLite query times and scanned rows, image decode/hash/encoding time, lock hold time, cache hit rate.
- Retries, partial failures, duplicate reprocessing and memory peak.
- Accuracy guardrails: correct tags/identities, no raw-source mutation, no unintended deletions, no lost user edits or stale annotations.

First optimization hypotheses, not unmeasured claims:
1. SQL-level indexed keyset paging + lightweight projection instead of reading all media/artifacts on every page/view.
2. Cached asset metadata, derived thumbnails/contact sheets, pHash/DINO embeddings, invalidated by revision.
3. One evidence-grounded model pass per eligible image instead of multiple analysis passes; no model call for deterministic tasks.
4. Staged parallel preprocessing, brief write locks, and asynchronous long-running jobs.
5. Batch Agent outputs using typed, capped payloads with per-image result accounting.

## 8. Safety & release gates

- Maintain full-image-before-mutation, exact-crop-preview-before-apply, current-hash/stale-check, verified-original retention and 32 alignment.
- Visual duplicate similarity never authorizes deletion; rejects go to a human-confirmation queue.
- Model and image transfers to external Agent/vision providers must follow an explicit user authorization policy and be observable.
- Cancellation, crashes and resumes must never duplicate deletion/overwrite effects.
- Add contract tests for CLI/MCP parity, pagination under large catalogs, multi-process locks, cancellation, lease expiry, stale revision, audit, source retention and failure recovery.
- Test with synthetic images and isolated catalogs, then explicitly authorized real-world smoke tests. Require Windows/Linux checks.
- Migrate incrementally; existing MCP clients must not break before a versioned replacement exists.

## 9. Discussion checkpoints (OPEN; deliberately undecided)

1. **Scheduler lifecycle: A accepted.** Independent persistent daemon; CLI, MCP and GUI reconnect to it. Remaining: lazy-start policy, work-lease ownership and service packaging (ADR-001).
2. **Work allocation: A accepted.** Agents actively claim leased work via CLI/MCP (ADR-002). **Workflow requirement recorded:** tasks may run one independently declared node or an entire explicitly composed graph (ADR-003). Granularity, sizing, receipt schemas and workflow defaults remain to be finalized.
3. **Interface priority:** which capabilities belong to CLI and MCP v1; how many tools to expose to avoid unnecessary Agent context.
4. **Image presentation:** preview/contact-sheet grouping and when a full view is mandatory; token vs fidelity budget.
5. **Quality policy:** rejection/duplicate thresholds, preservation of composition and style, and human override.
6. **Deployment and resource budgets:** Windows/Linux, model/provider capabilities, number of parallel Agents, CPU/GPU/network contention.
7. **Benchmark targets and acceptance criteria:** agree on real samples before setting throughput or latency promises.

## 10. Proposed PR sequence (subject to discussion)

- Stage A: API/contract decisions, resource budget and measured baseline.
- Stage B: shared use cases + standalone headless CLI + compatibility tests.
- Stage C: durable job scheduler, deduped work and batch mechanics.
- Stage D: MCP adapter/tool refactor, contact sheets and efficient Agent loop.
- Stage E: GUI handoff, observability, regression tests and performance validation.

This draft PR should remain unmerged until the contract and scheduler decisions are reviewed. Follow-on implementation may be split into smaller PRs to control regression risk.
