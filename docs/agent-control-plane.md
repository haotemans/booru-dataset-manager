# Headless CLI and durable Agent work (v3)

The CLI `bdm.exe`, desktop MCP adapter and desktop task controls use the same local operations and persisted jobs. Vision/reasoning stays with the external Agent. The daemon never calls a hosted annotation model. Existing desktop download and export use cases are reused.

This implementation uses SQLite transactions plus a catalog-specific OS daemon lock as its local control plane. It does **not** implement the RFC's named-pipe/UDS RPC transport, distributed workers or a general resource scheduler. The old standalone MCP tools remain compatible.

## Start and inspect

```powershell
$bdm = 'D:\project\booru-dataset-manager\dist\bdm.exe'
$common = @('--catalog', 'ABSOLUTE_CATALOG_PATH', '--project-root', 'ABSOLUTE_PROJECT_ROOT', '--library-root', 'ABSOLUTE_LIBRARY_ROOT')
& $bdm --help
& $bdm policy @common
& $bdm doctor @common
& $bdm daemon-serve @common --allow-writes
```

Substitute the actual existing paths. `init --allow-writes` explicitly initializes a new catalog; ordinary reads never create a missing database. `--allow-writes` also installs additive v3 tables on an existing schema-v6 catalog. Without it, the CLI and MCP are read-only. All JSON results go to stdout; failures exit 2. No desktop window is started by the headless binary. The two binaries are different products, not duplicate backups.

The daemon runs in the foreground so a supervisor can own its lifetime. On Windows, launch background instances with `Start-Process -WindowStyle Hidden` and an argument list; do not interpolate untrusted JSON into shell commands. `daemon_stop --allow-writes` requests a drain after the current deterministic operation. A second daemon for the same catalog exits with an ownership error. Independent clients can continue listing images while inference runs.

`doctor` checks catalog/library access, daemon heartbeat, source credential **presence**, and default DINO weight presence. It never returns keys. Input `{"probe_runtime":true}` additionally probes Python plus torch/timm/Pillow/numpy with a five-second timeout, without importing the model or using the network. Presence is not a model checksum or a successful source-network probe.

`mcp-serve` exposes the same tools over stdio. The existing desktop `--mcp-server` command remains valid, so existing Agent configuration does not need its executable path changed.

## Typed tools and schema discovery

When policy advertises `typed_work_tools`, prefer these typed MCP adapters. They call the same operations and persisted state; `bdm_agent(operation,input)` remains compatible. Every typed tool exposes its full argument schema in `tools/list`, including nested regions, enums and required fields. Do not call `describe` before using these tools.

| CLI operation | MCP tool / arguments |
| --- | --- |
| `job_plan`, `job_submit`, `jobs_list`, `job_status`, `job_control`, `jobs_tick` | `bdm_job(operation,input)`; input is typed for each operation |
| `download_submit`, `download_status`, `download_control` | `bdm_download(operation,input)` |
| `work_claim`, `work_view`, `work_preview`, `work_apply`, `work_assess`, `work_annotate` | Corresponding `bdm_work_*` tool, direct arguments without operation/input wrapper |
| `work_heartbeat`, `work_release` | `bdm_work_control(action="heartbeat"/"release",work_id,lease_token,generation)` |

Work payloads exclude server-owned identity/hash, overwrite, assessment stage and apply policy. Crop preview requires `expected_sha256` and payload, but no `request_id`; applying, assessing and annotating also require a unique write request ID. `next.tool` names the preferred adapter; `next.operation` remains the CLI/legacy operation. Existing standalone tools retain their behavior.

`bdm schema` lists operations, and `bdm schema work_annotate` returns the complete envelope/payload JSON Schema without opening a catalog, reading credentials or creating files. `describe` also includes `input_schema`. These schemas describe typed envelopes; CLI display defaults remain legacy PNG, so pass preview format explicitly when choosing display options.

## Compound work (capability `compound_work`)

Three optional typed MCP tools and matching CLI operations reduce client round trips while reusing the existing stage, lease, image-hash and write checks:

| Operation / MCP tool | Behavior |
| --- | --- |
| `work_next` / `bdm_work_next` | Claim one item and return its current image plus canonical `work` write envelope. Supply owner, optional job_id/ttl_seconds, unique request_id and optional top-level preview. |
| `work_apply_view` / `bdm_work_apply_view` | Existing apply followed by full-current view; payload is `{crop,preview?}`. Crop is the exact inspected proposal, including its native preview SHA. Inspect returned pixels before final judgement. |
| `work_finish` / `bdm_work_finish` | Existing final assessment followed by requested annotation for KEEP. Payload is `{assessment,annotation}`; final crop_candidate=false. REJECT/DEFER forbid annotation and retain existing human-review/deletion rules. |

For annotation-only work use next → inspect → annotate; do not add unrequested curation. For full curation without a crop, next → inspect → finish replaces separate claim/view/assess/annotate calls. Crop work remains next → inspect → preliminary assessment → crop preview → inspect preview → apply/view → inspect saved current image → finish. A harmful preview may instead be finalized with verified `crop_outcome=not_improved`. Applying and finalizing cannot bypass preview evidence or actual final-image delivery.

Claims optionally accept request_id (mandatory for work_next). Claim selection and its receipt commit in one SQLite transaction. Identical retries return the same snapshot, not another lease; owner/scope/effective lease-policy changes conflict. Empty claims remain empty on replay even after preflight advances: use a new ID once ready. Plain work_claim replays the original snapshot; work_next rejects a released/expired lease during its follow-up view. Neither silently reclaims. Existing callers without a claim ID retain ordinary claim behavior. Metadata `claim.items` is the original snapshot; `work` contains only work_id, lease_token, generation and the current expected_sha256.

Finish binds the entire request to its lease and uses stable child assessment/annotation receipts. If interrupted between phases, identical retry resumes/replays existing child outcomes. **This is ordered, recoverable execution, not atomic rollback.** Assessment may already have saved when annotation fails; inspect `assessment_saved`, both results, `next_stage` and `annotation.readback`/`verification`. Correct rejected/missing outputs with work_annotate; do not rerun approved work or change a prior request ID's input. Busy/ambiguous writes keep the existing recovery rules. Tag-coverage validation failures that saved nothing now retain the annotation-stage lease for correction.

If apply saved but subsequent view fails, the response remains an error with `applied=true` and `apply_result`. Resume image inspection under a valid lease; never assume the crop was rolled back. The two compound image operations always return pixels when successful, default to JPEG and require CLI `.jpg`/`.jpeg` output unless preview.format is explicitly PNG. Their previews never change original/crop/export pixels.

## Display previews

Typed MCP work tools default to longest edge 1024 and JPEG quality 90. Legacy tools/CLI operations default to PNG; the two new compound CLI image operations default to JPEG. Set `preview` on standalone view/crop arguments or `payload.preview` on work view/preview: `{"max_edge":2048,"format":"png"}`. The edge range is 256–2048; small images are never upscaled. Use PNG or detail tiles for fine text. These options affect Agent viewing only: originals, native crop PNGs and training export are never resized or recompressed by preview transport.

Each response reports native/display dimensions, MIME type, byte length, encoded SHA, a source-bound `display_preview.token`, cache hit and `not_modified`. An in-memory LRU holds at most 32 previews / 32 MiB, shared across leased work. Source hashes/path boundaries and fresh metadata are still checked before cache use. No cache image files or backup copies are created. Exact native crop proof (`preview_sha256`) is separate from display hash/token and verified again on apply.

Supply `known_preview_token` only when the client retains and has actually inspected that exact full preview. A repeat full-current view under an already-viewed lease may return `not_modified:true` without an image block. Omit the token to fetch pixels again. A new lease, changed current SHA, original/detail tile and crop preview always send an image. Cropping clears the inspection receipt; the saved current image must be delivered/seen again. Cache hits alone are never inspection evidence.

MCP uses image content blocks with the actual MIME type, not Base64 in a text field. CLI writes only to an explicit unused absolute `--output` path: `.png` by default or `.jpg`/`.jpeg` with `format:"jpeg"`. JSON includes `image_file` and `image_mime_type`. A path/hash alone is not inspection; the calling Agent must open that file.

## A durable batch

Pass a JSON file with `--input ABS_FILE` or stdin with `--input -`. CLI image operations additionally require an unused absolute `--output IMAGE` path: actually inspect that file before submitting a visual judgment. Files cannot be silently overwritten. MCP delivers the display image directly.

Example `job_plan` / `job_submit` input:

```json
{
  "request_id": "curate-my-collection-001",
  "mode": "curate",
  "collection": "EXACT_LOCAL_COLLECTION_TAG",
  "outputs": ["caption", "tags", "regions"],
  "overwrite": false
}
```

Use `media_ids` **or** `collection`, never both. A submission snapshots 1–10,000 IDs, image hashes and annotation revisions. Modes are `curate`, `annotate`, `bucket`, `export`; each remains independently usable. `job_plan` has no effects and returns `plan_sha256` plus at most 16 preview items by default (`include_items:true` explicitly requests the full snapshot). Optionally pass it as `expected_plan_sha256` on submit to detect changes between preview and submission. Submission retries use the same request ID and the exact original input; a changed input requires a new request ID. Replays retain the original scope, including after the job itself edits images or text.

1. Submit once. A running daemon performs local preflight; otherwise call `jobs_tick`. Already-complete requested outputs are skipped in normal annotation continuation. Original image files are verified before work is offered.
2. `work_claim` with `{"owner":"actual-client-name","job_id":123,"limit":4}` returns up to 16 independent work items. Each includes `work_id`, `lease_token`, `generation`, `expected_sha256`, `stage` and expiration. Treat tokens as local credentials, not captions or logs.
3. Use `work_view` on each full current image. Follow `next.tool` / `next.operation` through `work_assess`, `work_preview`, `work_apply`, `work_view`, final `work_assess` and `work_annotate` as applicable. Prefer typed MCP tools, whose complete arguments are already in `tools/list`. Older clients may retain `bdm_agent` and call `describe` once for an unfamiliar payload.
4. Every work call carries its work ID, token and generation. Writes also carry a unique `request_id`, the inspected `expected_sha256`, and a `payload` matching the corresponding standalone schema **without** media ID/hash. The server owns media identity, stage, overwrite policy and final-assessment requirements. A view of the original or a small tile does not replace seeing the full current image.

Example write envelope:

```json
{
  "work_id": 456,
  "lease_token": "TOKEN_FROM_CLAIM",
  "generation": 1,
  "expected_sha256": "HASH_FROM_VIEW",
  "request_id": "annotation-456-001",
  "payload": {
    "caption": "Factual English visual prose without identity names.",
    "tags": ["solo"],
    "text_regions": [],
    "visual_regions": [],
    "special_elements": [],
    "reason": "Observed the final current pixels",
    "model": "actual-client-model-or-unknown"
  }
}
```

Empty region arrays assert an actual inspection found no such elements; do not fabricate an empty inspection. Identity formatting, variant merging and artist-first prefixes remain local. Raw source data is immutable. Regions-only jobs may save regions without touching existing training TXT. Corrections may omit fields already saved in earlier calls; completion is checked against persisted outputs, not just the latest response.

Full curation routes preliminary crop candidates through a preview before a final decision. Applying a crop clears the full-image inspection receipt; the new saved PNG must be viewed again before final assessment. Final KEEP is required before full-curation annotation. Rejection enters the existing human deletion queue and never directly deletes image files.

## Lease, failure and recovery rules

- Default lease TTL is 600 seconds, configurable 30–1800 on claim. `work_heartbeat` renews it; `work_release` releases it. Expiration, pause and cancel fence old tokens/generations. Standalone mutation tools cannot bypass an active work lease.
- `job_control` uses `job_id` and action `pause`, `resume`, `retry`, `cancel`. Pause fences idle work immediately; a write already committing may finish. Cancel prevents subsequent stages and cannot be resumed. Resume never replays an already-completed effect.
- A repeated write with the same request ID/payload returns its original receipt, including validation errors. Change the request ID for a corrected payload. Another lease cannot obtain that receipt using a guessed request ID.
- Detected image or annotation/sidecar changes produce a conflict and preserve newer data, including when overwrite was authorized at submission. Inspect current data and submit a fresh plan when the snapshot is obsolete. Protected stale annotations are deferred rather than counted as newly generated success.
- The implementation journals intent before a write. If the process dies between the effect and acknowledgment, `recovery_required` blocks automatic retry: inspect current images/TXT and receipts, then submit a reviewed new plan. This is deliberately conservative; it is **not** an exactly-once filesystem transaction or an automated repair engine.
- Finished/failed/deferred jobs leave the dashboard progress area; `jobs_list` and `job_status` retain their history and retry information. `job_status` pages work via `after_id` / `limit` (default 64, maximum 128), returns full stage counts and compact per-item results, and includes the full immutable scope only with `include_plan:true`. Interrupted local workers have a five-minute renewable deadline. No completed image or existing TXT is rolled back just because its parent job paused.

## Buckets and exports

`bucket_plan` accepts `media_id` and returns candidates for the existing ratios only. It chooses among 768/1024/1536/2048 using native area and checks actual `BucketManager(no_upscale=True, reso_steps=32)` geometry. It does not automatically force 1024. The Agent still chooses composition after seeing the image.

Leased crop previews include the exact native-pixel selection, recommended tier and trainer resize geometry. Any extra trim needed to avoid a subsequent Kohya edge crop is shown **before** apply. Apply saves that exact selection as native-pixel PNG, preserves the complete original and never downsamples the stored image. If this trim harms the expression, keep the original and report `not_improved` after inspecting a preview. `training_resize_required=true` means the trainer will resize for that tier; no-upscale does not mean no downscale. The guarantee applies to the reported tier and no-upscale/32 settings, not every possible training configuration. Legacy crop/restore records retain their earlier geometry semantics.

The Rust geometry is tested against 44 outputs generated from the user's local `sd-scripts/library/dataset.py`; the fixture records its SHA-256. Native flat export remains the default: source-sized current PNG and existing selected TXT, with managed ownership checks and a single dataset annotation index. Export mode additionally requires absolute `destination` and `text_kind` (`caption`/`tags`). Explicit `export_layout:"multi-resolution"` opts into the existing separate-resolution export; it does not replace flat export or resize the library original. Export the whole selected batch once, not one new export/index per image.

## Downloads and standalone dedup

`download_submit` accepts a unique `request_id`, an existing `collection_id`, `source` (`automatic`/`danbooru`/`safebooru`/`gelbooru`), and `minimum_short_side` (default 800). `safedanbooru` is an alias for `safebooru`: the fixed `safebooru.donmai.us` General-only entry, sharing Danbooru's IDs and account, with no source fallback. It queues the desktop ingestion use case; only daemon execution or `jobs_tick` starts network requests. Source credentials stay in the native credential store with the same portable/global account scope as the desktop. `download_status` and `download_control` use `download_id`; controls support pause/resume/retry with saved cursors and failed items, including the persisted safe-entry mode. Desktop controls route these managed downloads through the same queue.

`bdm_scan_duplicates` / `bdm_resolve_pair` remain standalone MCP tools. DINO/pHash inference runs outside the media mutation lock; current hashes are revalidated before a short feature/candidate commit. Scans never auto-delete images. Pair decisions still require viewing both images in the same MCP session and use the existing final human deletion queue. The durable curation job does not yet schedule visual duplicate-pair work itself.

## Validation boundaries

Tests use temporary catalogs/images, verify cross-client continuation, lease fencing, stale/manual-text conflicts, idempotent writes, crop-first ordering, native PNG geometry, regions-only saves, download queue semantics, CLI JSON behavior and unchanged desktop workflows. They do not invoke a paid model, download a user's gallery or alter the real dataset. Windows CI builds and publishes both executables.

## Visual crop proposals and retained element boxes

Agent crop calls require one of policy.crop_ratios, a visually chosen bbox and a composition-specific reason after full-image inspection. The preset planner keeps nominal aspect error within 2% on the 32-pixel grid. Previews return the exact native selection and preview_sha256; durable apply must echo the same bbox/ratio/hash. A changed ratio or altered proposal needs a new inspected preview. Pure native-image alignment in a bulk loop is not an Agent visual crop plan.

Final KEEP reports retained_elements (text, watermark, sticker, overlay, logo, signature; [] only after actual inspection). Full-curation annotation validates matching BBox kinds before writing text. Plans with retained elements must include regions output. OCR uses text_regions; non-text watermarks/logos/signatures now also have visual_regions kinds. All boxes refer to the final current image SHA. The desktop More menu can display these boxes without changing image pixels and suppresses stale-SHA boxes and boxes over a different edit preview.

Prefer cropping a safely removable edge watermark. If removing embedded typography or a large watermark damages the subject or composition, retain the artwork and label each occurrence. The server validates proof/geometry/schema consistency; the external Agent remains responsible for seeing every occurrence and judging composition. The project Skill documents the exact decision rules; no hosted vision calls or image edits are automatically performed by installing this update.

## Annotation completeness

Image inspection returns `tag_vocabulary.baseline_tags` for comparison, not mandatory verbatim retention. Unchanged-image baselines include locally accepted ordinary source/current tags; edited-image baselines use outputs bound to current image/content hashes. Quality/admin terms, canonical duplicates and local identities are excluded. The Agent may remove redundant concepts and broader terms fully covered by retained specific ones while preserving independent visual details. The service blocks an abnormal reduction of at least 5 canonical baseline terms AND at least 30% of the baseline before any output is written (`tag_coverage_incomplete`). Padding with unrelated terms, reasons and overwrite flags do not bypass it. Allowed saves include exact `tag_changes` in their receipt and audit so the Agent can review lost concepts without another tool call. This numeric safeguard does not establish semantic redundancy; large valid simplifications require review rather than blind restoration. Stale pre-edit source tags are not mandatory on new pixels. No hidden auto-merge is performed.

Caption instructions require coverage of all important visible subjects, clothing/actions, scene/composition, light/color/rendering and text/compound regions, without a word-count target. Region coordinates remain structured metadata. File/schema validation does not certify semantic completeness. These checks reuse the existing view/save calls.

## Local character prefix repair

Artist downloads persist `StoredMedia.artist_scope` from the actual selected artist collection. Artist identity formatting, review/copy, API/MCP annotation and fresh export use those explicit targets; unrelated source artist tags do not create extra cards or training identities. An explicit download of the same bytes for another artist adds that actual target, including across providers. Character/query imports retain source-driven artist discovery while no artist target is selected. Original source artist metadata and raw TXT are immutable. Startup backfill uses recorded successful/reused artist-download outcomes, not inferred membership in auto-discovered cards; only empty automatic extra cards are cleaned up. The scope also participates in leased annotation revisions.

Explicit local `TagRules.excluded_artists` corrections suppress known incorrect source artist tags in normalized identities, review/export output and MCP artist metadata. Automatic source collection discovery/attachment respects the same exclusions. Original source records and raw TXT remain unchanged; numerical spelling alone never makes an artist invalid. The existing bounded identity repair also removes excluded leading artist tokens while preserving visual prose and unrelated identities.

Artist/character identities remain local. Confirmed base→variant relations apply to caption prefixes, optimized tags, copy/display, new writes and new exports; verified ancestor chains also work when an intermediate tag is absent. A base by itself and unrelated characters are retained. Relationships are never inferred from similar spelling or co-occurrence alone.

`character_repair` accepts explicit `media_ids` (1–64) and defaults to `apply:false` preview. With `apply:true` and `--allow-writes`, it repairs current caption/tag outputs under existing verified rules, preserves description-body bytes and reviewed/state/provenance flags, and records an audit event. It does not add character relationships, invoke a model, edit images or raw source data, or overwrite sidecars that differ from their recorded content. Active work leases block repair. Existing exported TXT preservation remains unchanged.
