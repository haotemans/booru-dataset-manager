# ADR-005: Exclude sequential comic pages from the illustration dataset; optional future structured comic labeling

**Status:** Partially accepted design direction (2026-11-15): for the current **illustration-oriented dataset**, comic/manga **story pages** need not be collected, aesthetically rated or exported. The owner raised a conditional alternative—if a useful, reliable comic-specific aesthetic+annotation pipeline can be designed, consider that separately. **No runtime behavior or destructive cleanup has been authorized.**
**PR:** #11, draft; architecture and contract design only.
**Related:** [ADR-003](AGENT_WORKFLOW_NODE_CATALOG.md), [ADR-004](AGENT_AESTHETIC_EVALUATION_WORKFLOW.md), [existing ingestion](../features/INGEST_PIPELINE.md), [caption/tag preservation](../features/CAPTIONS_AND_TAGS.md).

## 1. Target distinction

This policy is about the **content form of a narrative comic/manga page**, not an artist identity, monochrome palette or manga/anime drawing style.

| Detected category | Illustration dataset default | Rationale |
| --- | --- | --- |
| `standalone_illustration` | eligible | An anime/manga-styled single artwork is still an illustration |
| `narrative_comic_page` | **exclude** | May have multiple panels, balloons, captions, reading order and sequential context; no general-image aesthetic score |
| `storyboard_or_comic_strip` | **exclude** | Sequential narrative layout is unsuitable for the default single-illustration target |
| `uncertain_or_mixed` | **auto-quarantine for illustration export; optional human review** | No silent hard-delete or ungrounded aesthetic score; normal batch processing must not require manual triage |

One speech bubble, text overlay, monochrome drawing or more than one character **alone** is not conclusive comic evidence. Source tags (`comic`, `4koma`, `manga`, `multiple_panels`, etc.) are fallible hints and can be missing or misleading; verify the **content form** when uncertain.

## 2. Proposed default `exclude_comic_pages` policy for illustration curation

Apply a **content-scope gate** as early as reasonably reliable, distinct from format/animation filters, quality scores, keep/reject decisions and duplicate logic:

1. **Discovery prefilter:** if source metadata clearly and reliably identifies a sequential comic page under a calibrated policy, return a `source_excluded` receipt with reason `COMIC_PAGE_OUT_OF_SCOPE`; **do not download** or invoke costly Agent analysis.
2. **Ambiguous content:** query a size-bounded approved preview/classifier when available; if uncertain, record `needs_classification` and avoid an irreversible decision. Image transmission to an external model still requires explicit permission; do not create a hidden API charge. A human review can settle ambiguous cases.
3. **Post-fetch pre-import:** if a valid supported static image was temporarily downloaded and is confirmed comic content before import, record the excluded source reference and safely delete the **uncommitted temporary download**, not an existing managed original. Distinguish this from a fetch or file integrity failure.
4. **Existing managed media:** for an illustration-scope job, mark a **dataset-scope exclusion** in the plan/result and omit from subsequent illustration aesthetic analysis, caption work **for that scope**, training export, and representative ranking. Do **not** convert exclusion into `media.keep=false`, duplicate rejection, deletion queue entry or global metadata mutation. It may remain needed by other collections/workflows.
5. **Cleanup is separate:** an explicitly requested library-space cleanup can enumerate confirmed comic-only, unreferenced managed images with a dry-run showing shared associations, prior reviewed text, other jobs, originals and bytes saved. **Physical deletion requires separate explicit user confirmation and existing audited deletion protections.** Never delete shared media or silently delete based on text tags.
6. **Accounting:** report `excluded_comic`, `uncertain`, `eligible_illustration`, and `failed` separately. Excluded is neither a failed download nor a low aesthetic score; source API total counts remain the source's totals. Do not keep re-processing known reliable exclusions.

Proposed fast paths: free deterministic metadata checks first; local layout heuristics (panels/bubbles) as supporting signals second; **Agent vision should resolve remaining uncertain cases automatically, with no routine manual triage step**. If vision is unavailable or still uncertain, the image remains held out of the illustration export and separately reported for optional review. Record whether the decision was metadata-, image-, or human-verified; attach classifier/model version, current SHA, policy version and reason. Avoid guaranteeing comic detection accuracy without a labeled evaluation set, and optimize for very low false positives when discarding downloads.

For standalone `node.run aesthetic.evaluate`, do **not** assume an entire ingestion workflow: an existing confirmed comic receives `skipped / UNSUPPORTED_AESTHETIC_FORMAT`. A composite illustration workflow can use this exclusion gate upstream. No new implicit model call or dataset deletion is allowed from a single-node command.

### Illustrative exclusion receipt (not implemented)

~~~json
{
  "source_ref": {"provider": "danbooru", "post_id": "opaque-id"},
  "media_id": null,
  "category": "narrative_comic_page",
  "decision": "exclude_from_illustration_dataset",
  "reason_code": "COMIC_PAGE_OUT_OF_SCOPE",
  "confidence": "high",
  "evidence_source": "verified_content_or_metadata",
  "aesthetic_score": null,
  "permanently_deleted": false
}
~~~

## 2A. Automatic comic filtering without routine human screening (revised proposal)

**Product correction:** Exact Danbooru tag \`comic\` is a **cheap first-stage filter, not a reliable complete filter**: user uploads may omit or misuse the tag. The owner does not want to manually screen normal ingestion batches. The scheduler therefore needs a **conditional automatic content inspection** in addition to tag filtering, while still protecting valuable standalone illustrations from false-positive deletion.

**Fastest default illustration curation:**
1. Apply source query exclusions where supported (e.g. \`-comic\`) and verify the raw/general tag token list locally using exact match. Persist reason \`COMIC_TAG_EXCLUDED\`; no Agent call or file download for definite tag hits. Tag aliases/implications must not be assumed to be available unless verified from the source.
2. For non-tagged media that **already undergo Agent vision for aesthetics/caption**, include a concise required field in the **same image understanding pass**: \`content_form = standalone_illustration | sequential_comic | uncertain\`, \`confidence\`, and a short \`evidence\`. **This avoids a second model call** for the common full-curation workflow.
3. Any confident \`sequential_comic\` result is marked \`COMIC_PAGE_OUT_OF_SCOPE\` and excluded from aesthetic ranking, annotation *for the illustration dataset* and export; never auto-delete the managed source.
4. If a **download-only** job must also avoid downloading untagged comics, image content cannot be inferred from tags alone. The optional \`strict_pre_download_comic_filter\` uses an available permitted remote thumbnail/preview and a vision classifier before original download. If no usable preview exists, either (a) download to a bounded temporary file and classify before import, or (b) defer visual classification until the next vision curation stage. This is a configurable cost/coverage trade-off, not a free guarantee.
5. An inconclusive visual result is \`uncertain\`. **Do not ask a person to inspect every uncertain item by default**. Keep the image in a review/quarantine set and exclude it from automatic illustration export until a later Agent re-check or optional user review. No quality-zero score, Keep/Reject change or permanent deletion. Avoid unbounded repeated reclassification.
6. A single panel-styled illustration, manga cover or monochrome standalone art remains eligible. Classify **sequential narrative content form**, not black/white palette or artist style.

**Default and optional modes (proposed):**
- \`curation_auto\` (recommended): tag early; fuse automatic image-form classification with existing Agent aesthetic/caption reasoning; no routine human review; hold out uncertain records from export. May temporarily import untagged comics before classification; that is a throughput/storage trade-off.
- \`download_fast\`: tag early only; very fast, but **does not claim complete comic exclusion**. Pending images have \`content_form=unverified\` and must be checked before strict illustration export.
- \`download_strict\`: tag early + visual preview classifier before retaining original; more compute/vision calls and consent required; minimizes downloading obvious untagged comics but cannot guarantee perfect classification.
- \`human_review\`: optional corrections and threshold calibration, **not a mandatory normal processing step**.

**Suggested fused Agent output (illustrative):**
~~~json
{
  "media_id": 123,
  "content_form": "sequential_comic",
  "confidence": "high",
  "evidence": ["multiple ordered narrative panels", "dialogue balloons across panels"],
  "illustration_dataset_eligible": false,
  "decision_source": "agent_visual",
  "aesthetic_score": null,
  "physical_delete": false
}
~~~

**Acceptance gates:** Measure false-positive comic exclusions with manga-style *standalone* illustrations, covers, monochrome scenes, character sheets and comics deliberately missing \`comic\` tags. Compare automatic classification field added to existing Agent pass against a separate classifier in model calls, image tokens, accepted throughput, and residual comics in exports. A system without vision execution must honestly mark unverified records rather than claiming full filtering. Automatic classification remains advisory for metadata; it can deterministically exclude from the selected illustration export, but **never** authorize permanent library deletion.

## 3. Comic labeling **is feasible** but is a different target and should not be bundled with v1 illustration curation

A specialized comic flow would use page-and-panel structure rather than one single whole-image caption:

1. **Page layout detection:** identify panel regions, page borders, separators and reading direction; associate overlapping/inset panels.
2. **Reading order:** infer panel order and document ambiguity (for manga commonly right-to-left; for other comics often left-to-right; artistic layouts can override both).
3. **Text detection and attribution:** detect text/bubbles, narrator boxes, captions and sound effects; OCR or transcribe only legible text; associate balloon tails with speakers only if supported. Do not invent unreadable dialogue.
4. **Per-panel visual grounding:** caption subjects, visible poses/actions, environment, expressions, interactions and changes; maintain consistent character references **where actually supported**.
5. **Cross-panel events:** optional short page-level sequence summary based on evidence in visible panels only. Avoid inventing unseen before/after plot events; neighboring-page context is necessary for richer storytelling interpretation.
6. **Structured tags:** page-level `monochrome/color`, `panel_count`, `reading_direction`, `speech_bubbles`, `narration_boxes`, `onomatopoeia`, `screen_tone`, `panel_layout`, `genre` (only if supported), plus **per-panel** entities/actions/scenes. Artist/character identity categories remain protected and source-verified.
7. **Separate comic-specific quality:** technical readability, panel composition, visual continuity, line/screentone treatment, bubble placement/readability, storytelling clarity and visual pacing, each with calibrated human references. Do not import the illustration 0-10 score as a universal comparison. Narrative aesthetics ideally evaluate a page sequence, not a single isolated page.

Example **annotation-only** shape:

~~~json
{
  "format": "comic_page",
  "reading_direction": "right_to_left",
  "panels": [
    {
      "panel_id": 1,
      "bbox_1000_xyxy": [510, 20, 970, 480],
      "reading_order": 1,
      "caption": "A character stands beside a window, looking outside.",
      "visible_tags": ["window", "standing", "side view"],
      "speech_bubbles": [
        {"bbox_1000_xyxy": [720, 40, 920, 170], "text": null, "legible": false}
      ]
    }
  ],
  "page_summary": "The visible sequence opens with a character near a window.",
  "aesthetic_score": null
}
~~~

Illustrative only: coordinates/text are not observations of an actual image. No extraction pipeline, manga segmentation/OCR weights, or labeled aesthetic model are presently shipped. Each proposed new field requires source/target compatibility, versioning and output tests before support can be claimed.

Public research validates that structured **layout annotation** is possible but does not imply reliable **general aesthetic scoring**: Manga109 includes panel/frame, character face/body and text annotations; CVPR 2025 Manga109 segmentation adds balloons and onomatopoeia; ICCV 2025 Magiv3 uses panel/character/bubble/text association, reading order, OCR and panel captions. These are reference designs, not installed dependencies or a license to redistribute their images.

- Manga109 annotation scheme: https://manga109.github.io/manga109-project-website/en/annotations.html
- Manga109 segmentation, CVPR 2025: https://openaccess.thecvf.com/content/CVPR2025/papers/Xie_Advancing_Manga_Analysis_Comprehensive_Segmentation_Annotations_for_the_Manga109_Dataset_CVPR_2025_paper.pdf
- From Panels to Prose (Magiv3), ICCV 2025: https://www.openaccess.thecvf.com/content/ICCV2025/papers/Sachdeva_From_Panels_to_Prose_Generating_Literary_Narratives_from_Comics_ICCV_2025_paper.pdf

**Recommendation:** do not build/enable a comic Profile or specialized flow in PR #11. **Exclude comics from the current illustration training target**, retaining the option to define a separate, explicitly authorized comic dataset later.

## 4. Proposed CLI/MCP integration

**Generic node proposal:** `content.classify` (classification of illustration vs sequential comic/uncertain) is a **separate Node Definition**, with a cheap local source-metadata preflight and an optional Agent vision substage only when needed/consented. A single-node execution does not automatically delete, download or import other media.

**Composite workflow:** the illustration pipeline places the content gate before full download where evidence suffices and repeats validation before import/export as needed. Future `comic.page_annotate` and `comic.page_aesthetic` would be **opt-in and absent from v1 tool discovery**.

- CLI: `bdm node run content.classify --input scope.json --json`; `bdm workflow plan --profile illustration-curation --json` returns exclusion counts, uncertain counts and any review requirement.
- MCP: `bdm_node_run`, `bdm_work_lease`, `bdm_work_view`, `bdm_work_submit`; new classified outputs use the same strict lease, revision and idempotency contracts.
- Compact `source_excluded` results should not include model reasoning/images unless specifically requested. Support inspecting the source decision/evidence when needed.
- Do not overload `bdm_assess`: it currently requires a Keep/Reject/Defer decision and can queue deletion. **Out-of-scope content is an eligibility decision, not an aesthetic or deletion decision.**

## 5. Benchmark, policy and test gates

- Measure content-gate false positives/false negatives on varied comics, manga cover art, monochrome illustrations, 4-koma, character sheets, posters, visual novels, splash pages, text overlays and single-scene panel-styled illustrations. Prefer uncertain review to losing valuable illustrations.
- Assert `exclude_comic_pages` excludes confirmed comic pages from **new ingestion and illustration export**, but does not mutate shared originals, other collections, human reviews or already exported data without a separate owned-output refresh.
- Confirm future pagination and incremental sync do not repeatedly download known excluded references. Persist only source skip metadata appropriate for the collection policy.
- Assert no score is generated for excluded comic pages; null never becomes zero for sorting/training.
- Only explicitly requested, reviewed library cleanup may permanently delete managed media after association and version checks. Existing legacy dedup behavior must not be invoked as a surrogate cleanup action.
- All new classifications and nodes must be explicitly tested and measured before claims of improved speed or accuracy.

**Open decisions:** whether to prefer `curation_auto` or `download_strict` by default; the acceptable false-positive/false-negative rates; and future optional comic dataset scope. Default routine human screening is **not** a requirement. Permanent cleanup remains a separate, explicitly authorized operation.
