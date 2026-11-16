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

## A durable batch

Pass a JSON file with `--input ABS_FILE` or stdin with `--input -`. CLI image operations additionally require an unused absolute `--output PNG` path: actually inspect that file before submitting a visual judgment. Files cannot be silently overwritten. MCP delivers the PNG directly.

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
3. Use `work_view` on each full current image. Follow response `next.operation` through `work_assess`, `work_preview`, `work_apply`, `work_view` and `work_annotate` as applicable. `bdm_agent` wraps these operations for MCP. `describe` with `{"operation":"work_annotate"}` returns the exact payload schema once; reuse it for the batch.
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

`download_submit` accepts a unique `request_id`, an existing `collection_id`, `source` (`automatic`/`danbooru`/`gelbooru`), and `minimum_short_side` (default 800). It queues the desktop ingestion use case; only daemon execution or `jobs_tick` starts network requests. Source credentials stay in the native credential store with the same portable/global account scope as the desktop. `download_status` and `download_control` use `download_id`; controls support pause/resume/retry with saved cursors and failed items. Desktop controls route these managed downloads through the same queue.

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
