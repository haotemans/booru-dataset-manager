# ADR-004: Aesthetic evaluation as a first-class independent Agent workflow

**Status:** PARTIALLY ACCEPTED. The owner approved **shared aesthetic dimensions + style-specific, versioned Profile extensions** and decided **to defer comic/manga aesthetic assessment** on 2026-11-15. Concrete scoring calibration, profile auto-selection, execution defaults and review decisions are still **proposals pending discussion**.
**PR:** #11, draft / docs only; no runtime or existing review decisions changed.
**Related:** [ADR-003: composable workflow and node catalog](AGENT_WORKFLOW_NODE_CATALOG.md), [ADR-002: Agent-pull leases](AGENT_PULL_LEASE_PROTOCOL.md), [existing visual identity design](../features/DINOV3_VISUAL_IDENTITY.md).

## 1. Scope and distinction

Aesthetic quality is a **separate first-class process**, not a synonym for technical file quality, training suitability, perceptual similarity, resolution or popularity.

- **Aesthetic evaluation**: overall artistic appeal and visual intent based primarily on the current image, with calibrated, subjective explanations and uncertainty.
- **Technical validation**: image integrity, pixels, format, aspect ratio, cropping constraints and corruption. Local, deterministic; cannot infer beauty.
- **Training suitability**: fitness for a named training goal (style learning, character images, pixel art, motion, etc.), including diversity, annotation accuracy and policy-specific coverage. A beautiful image may be unsuitable for a particular target dataset, and vice versa.
- **Duplicate matching**: pHash/DINOv3 candidate generation is not an aesthetic score. Similarity/identity does not justify rejection or representative selection.

The owner can run **only aesthetic evaluation** on one image or a collection, include it as a child workflow inside full curation, or fuse its visual reasoning with caption/tag/quality assessment **without merging the persisted output fields or forcing unrelated writes**. No hidden download, crop, caption generation, deletion or export for a standalone aesthetic request.

Suggested workflow ID: **W16: Aesthetic Judgment**.
Suggested reusable node ID: **N23: aesthetic.evaluate**. It is Agent-executed through a normal pull lease, not a daemon-owned hosted LLM API.

## 2. Workflow inputs and contract

Inputs:
- Existing managed media IDs / collection query with stable snapshot and cursor, current SHA-256/revision and availability of a full-frame current image.
- Score profile ID/version, comparison mode (absolute vs explicitly eligible group), selected output fields, optional style/context hints, calibrated rubric/model revision.
- Agent capability (image vision), worker identity/lease, approved image-transmission policy, permission to write an aesthetic report.
- Optional human-confirmed comparable variant group ID and evaluation target for *relative ranking*. A broad same-character/same-style group is **not** automatically an interchangeable comparison group.

Preconditions:
- Valid readable current image and current SHA, permitted Agent image viewing, active lease (for commit) and no conflicting human edit. Inspect full-current-frame image before submitting a mutation/result.
- Metadata, source raw tags and artist/character identities can be read as contextual hints but must not override visible evidence.
- Never create new external model API calls inside daemon; the claiming Agent supplies vision interpretation.
- Aesthetic-only run must not alter current caption, normalized tags, keep/reject/defer, duplicate relations, crops, training flags, original file or existing human representative choice.

Outputs:
- Immutable/versioned report with per-dimension 0..10 scores or null, overall aesthetic score or null, rubric profile/version, Agent/model provenance, evaluation timestamp, current image SHA, confidence/uncertainty band and grounded pros/cons.
- Optional non-destructive suggestions (e.g. framing improvement, human review, difference-set representative candidate). These do not execute edits or deletions.
- If unreadable/insufficient image, return explicit incomplete/blocked result; null is more truthful than fabricated scores.
- Batch operation returns per-image success/no_change/skipped/blocked/failed and request-level counts; preserve prior valid reports.

## 3. Proposed aesthetic rubric

Score **current visible composition** by reference to its intended style (anime, painterly, pixel art, minimalist, high detail, etc.), not by assuming that more detail, higher resolution or more realistic skin is always better.

| Dimension | What to judge | Explicit anti-proxy rule |
| --- | --- | --- |
| composition | focal hierarchy, subject balance, readability, pose/framing, intentional negative space | centered faces and tight crops are not inherently better |
| color_and_light | palette cohesion, color-temperature interplay, luminosity, depth and atmosphere | bright/saturated/blue/orange is not inherently better |
| artistic_execution | linework, rendering, visible texture, materials, edge handling, stylistic consistency | photorealism or high-frequency detail is not inherently better |
| completeness | whether the piece looks intentional and finished for its visual language; coherence of layers and scene | rough brushwork, blank backgrounds, simplified shapes may be intentional |
| expressive_impact | distinctive mood, gesture/story, style-specific appeal and artistic identity | avoid penalizing unusual but deliberate aesthetic choices |
| visual_distractions | intrusive watermarks, unintended text, artifacts, accidental visual clutter | typography, text, overlays and dense detail can be intentional; report separately |

A holistic `overall_aesthetic` (0..10) is **not** unconditionally the arithmetic mean of sub-scores; if weights are used, store the versioned profile and calibrated aggregation rule. `training_suitability` is a separate score with a separate policy; do not add it into the aesthetic score. Do not rely on nominal source dimensions, popularity/ratings, DINO similarity, artist name, expensive model identity or source priority to determine aesthetic appeal. Allow `null` and `uncertain` when inspection is inadequate.

For large libraries, use a calibrated human-rated reference set covering different visual styles, with pairwise preference labels and holdouts by artist/identity/group. Report rank consistency/agreement rather than assuming numbers from different models/prompts are directly comparable. Human overrides always win until explicitly reset.

## 4. Node execution and scheduler behavior

Aesthetic-only:
1. Agent/client issues `bdm node run aesthetic.evaluate --input ...` or the corresponding generic `bdm_node_run`; daemon creates a durable one-node job `awaiting_agent` and does not invoke a model itself.
2. Agent claims an appropriately scoped lease and receives IDs/revisions and minimal metadata; no pixel bytes or lengthy prompts in lease-list JSON.
3. Agent retrieves full current image through CLI/MCP image-view path. Contact sheet may prioritize candidates, but is not sufficient evidence for committed aesthetic evaluation.
4. Agent judges visible evidence, records dimensional findings and calibrated uncertainty; there is no requirement to generate tags or a caption.
5. Agent submits bounded result batch with lease/generation/idempotency/current SHA; daemon stores per-image report with revision CAS and audit. No implicit `Keep/Reject` update, no photo edits, no output TXT modifications.
6. Future workflow edges are triggered only if the user explicitly submitted a composite workflow; standalone ends after report persistence.

Composite full-curation:
- Node `aesthetic.evaluate` can **share one Agent visual reasoning pass** with `vision.curate` / `annotation` where the same current image and consent/profile apply. The output must remain logically separate, nullable, per-field selectable, auditable and revision-specific.
- Local preliminary integrity/format checks may skip images that cannot be viewed; a technical-fail image is not thereby labeled aesthetically poor.
- A subjective aesthetic report may feed `representative.rank` and `review.route` as a recommendation, never an unauthorized deletion or an automatic final representative choice for broad identity groups.
- After actual crop/edit, invalidate the **current-image-bound** aesthetic report for that revision and reevaluate only if the selected workflow requires post-crop evaluation. Retain the historical report for the older revision.
- An unchanged valid report can be reused when image SHA, rubric/profile, evaluation policy, relevant model version, and source hints match; never silently replace a human-reviewed assessment. Pairwise rankings may need recalculation if group membership or policy changes.

## 5. Illustrative payloads and API

Standalone CLI (proposed, not implemented):

~~~bash
bdm node run aesthetic.evaluate --input aesthetic-request.json --json
bdm work lease --job JOB_ID --capabilities aesthetic,vision --max-items 8 --json
bdm work view --lease LEASE_ID --item WORK_ID --kind current --output inspected.png
bdm work submit --input @- --json
~~~

MCP: `bdm_node_run`, `bdm_work_lease`, `bdm_work_view` and `bdm_work_submit` use the same domain contract. Specialized `bdm_aesthetic_evaluate` may be a convenience wrapper only if profiling justifies extra exposed tools.

Input:
~~~json
{
  "node_type": "aesthetic.evaluate",
  "node_version": 1,
  "scope": {"media_ids": [123, 124]},
  "profile_id": "artwork-balanced-v1",
  "mode": "standalone",
  "requested_outputs": ["overall_aesthetic", "dimensions", "reasons"],
  "comparison_group_id": null,
  "overwrite_human_review": false
}
~~~

Per-item Agent result (illustrative):
~~~json
{
  "work_id": "work-123",
  "expected_sha256": "current-sha256",
  "lease_id": "opaque-lease",
  "generation": 12,
  "idempotency_key": "unique-retry-key",
  "aesthetic": {
    "overall": 8.4,
    "dimensions": {
      "composition": 8.6,
      "color_and_light": 9.1,
      "artistic_execution": 8.4,
      "completeness": 8.0,
      "expressive_impact": 8.1
    },
    "visual_distractions": ["small visible watermark"],
    "confidence": "medium",
    "strengths": ["cohesive lighting", "clear subject hierarchy"],
    "weaknesses": ["some visual clutter near a corner"],
    "style_intent": "soft anime illustration",
    "suggested_actions": ["optional manual framing review"]
  },
  "decision": null,
  "caption": null,
  "tags": null
}
~~~

The result stays **advisory**. Suggestions do not translate into deletion, Keep/Reject, image crop or automatically regenerated captions. Client-provided model name is stored only as provenance, not authentication. The daemon enforces authenticated scopes.

## 6. Distinct uses of an aesthetic result

- **Absolute assessment:** give independent feedback for each image and show strengths/weaknesses; do not claim a universal calibrated ranking across art styles without evidence.
- **Selection for a training goal:** combine **separate** aesthetic rating and training suitability with an explicitly chosen dataset policy, diversity budget and human-selected exceptions. Avoid reducing a whole dataset to one average score.
- **Representative ranking:** only among source-/human-confirmed interchangeable versions or meaningful difference sets where the policy allows ranking; never collapse alternate poses, costume designs or scenes merely because the same character appears.
- **Crop proposal:** aesthetic score may flag a composition concern, but does not authorize or make an exact crop. Preview, current-hash and separate approval remain required.
- **Manual review:** human override, decision provenance and confidence are preserved; scores do not unilaterally delete any media.

## 7. Performance and review guardrails

- One Agent view/reasoning pass for overlapping aesthetic + caption/tag requests when model capacity permits; **do not require caption generation when only aesthetics is requested**.
- Metadata batches ~4–8 and image batches ~1–4 are only benchmarking candidates; cap pixel/byte/token budgets separately and measure real per-model latency.
- Per-image result CAS and independent batch success. Avoid global model/DB locks during Agent reasoning; persist only short journaled writes.
- Compare with user-labelled reference examples; track pairwise preference agreement, intra/inter-model scoring drift, percentage uncertain, rate of human overrides and p50/p95 throughput.
- Never optimize blindly for a high numeric score at the expense of style diversity, composition variants or the user's manual review choices.

## 8. Existing implementation compatibility

Existing `bdm_assess` in `crates/infrastructure/src/agent/{params.rs,tools.rs}` accepts optional `aesthetic`, `training_suitability`, `composition`, `detail` scores (0..10) and **requires** `decision: keep/reject/defer`; it writes review/keep state and may queue a rejected image for human deletion review. This is **not** a pure score-only interface. Aesthetic-only node v2 must not reuse this mutation contract without separating the subjective measurement write from Keep/Reject decision side effects.

Existing `docs/features/DINOV3_VISUAL_IDENTITY.md` establishes aesthetic appeal/completeness as primary representative-selection criteria, with human override and careful separation from similarity and group eligibility. It also documents a legacy strict two-signal auto-dedup deletion path. The new Agent workflow must not implicitly call that destructive path: policy compatibility requires explicit design.

## 9. Decisions already accepted and still to settle

- **ACCEPTED:** defer automated comic/manga aesthetics in v1; see section 11 for guardrails.


1. **ACCEPTED: common base rubric + style-specific versioned Profiles.** Pending: exact bundled Profile definitions, default-selection policy and calibrated weights.
2. Assessment behavior: score-only as default (recommended); whether to optionally emit non-binding keep/defer/review recommendations.
3. Confidence and fairness: when to request a second review or human confirmation, and when to refuse to compare dissimilar styles.
4. Full workflow combination: fuse aesthetics with one vision-pass annotation by default if selected; can be disabled independently.
5. Dataset curation thresholds and profile-specific representative ranking policy; never assume one cutoff applies across all art.

## 10. Accepted aesthetic Profile architecture; proposed operational contract

### 10.1 Accepted principle

All art types share one stable report schema and **baseline evaluation dimensions**, but the evaluation criteria must be interpreted through an explicitly versioned **style Profile**. The Profile changes the artistic questions, examples, and permitted explanatory signals; it must **not** redefine the meaning of core persisted fields, make unsupported identity claims, or implicitly authorize file operations.

Core dimensions:
- `composition`: compositional hierarchy, focal point, figure/background relationship, intentional framing/negative space.
- `color_and_light`: palette harmony, visual depth, lighting/color atmosphere, intentional contrast.
- `artistic_execution`: craft appropriate to the visual language (not "more pixels or realism is better").
- `completeness`: whether the execution is coherent and sufficiently finished **for its intended style**.
- `expressive_impact`: impact, emotional readability, distinctive visual intention.
- `visual_distractions`: **separate descriptive findings**, because text, blank space, strong texture, and overlays can be intentional.

Return `overall_aesthetic` as a holistic subjective 0-10 judgement (or null when unsupported), with separate optional dimension scores and evidence. There is **no adopted universal weighted formula or score cutoff**. `training_suitability` and any source technical-quality score remain separate artifacts.

### 10.2 Initial Profile candidates (proposed, not frozen)

| Profile ID candidate | Additional style-specific review | Avoid incorrect penalties |
| --- | --- | --- |
| `general-illustration@1` | figure/scene integration, composition, illustrative craft, color rhythm, atmosphere | deliberate stylization, sparse backgrounds, limited detail |
| `pixel-art@1` | pixel clustering, silhouette/readability, palette control, deliberate dithering, coherent grid and animation readability when applicable | low native resolution, limited shades, hard pixel edges, minimalist detail |
| `watercolor@1` | pigment/wash hierarchy, transparent layering, wet/dry edge control, paper texture and atmospheric transitions | bleeding/pooling or loose strokes that are deliberate |
| `galgame-cg@1` | narrative framing, character expression, pose/background consistency, scene lighting, design appeal | intentionally stylized anatomy, dialogue overlays or compositional framing when contextually intentional |

A future `collection:<id>/<profile>@N` or user-defined Profile is a preset of **declared evaluation preferences**, not an authority to rewrite protected artist/character names, original tags, image crops, or review decisions. Do not infer a specific copyrighted artist identity or source solely from image style.

### 10.3 Profile selection and comparison (proposed defaults)

1. In a standalone request the user/Agent may supply `profile_id` and `profile_version`. In a workflow, the profile is frozen in its immutable job snapshot.
2. If no Profile is provided, use a generic baseline Profile and record that fallback. Optional suggested style classification is **advisory**, never a silent authoritative switch, especially when styles are ambiguous or mixed.
3. Comparisons default to the same Profile, rubric version, model/prompt configuration and comparable review purpose. Do not rank values across disparate profiles/models as if they are globally calibrated.
4. A project-specific Profile may add *criteria* and user-expressed preferences, but cannot silently change the default deletion/review approval policy or discard original source.
5. A profile edit is a new version. Existing scores persist with their original profile, model, prompt/policy fingerprint, image SHA, and calibration identifier (if any); selectively reevaluate outdated items when explicitly requested.
6. Establish calibration samples spanning each style and difficult edge cases; compare with human paired-image preferences and record agreement/drift before adopting automatic cutoffs or weighted aggregate formulas.

### 10.4 Proposed typed input and output

~~~json
{
  "node_type": "aesthetic.evaluate",
  "node_version": 1,
  "scope": { "media_ids": [123, 124] },
  "profile": { "id": "pixel-art", "version": 1 },
  "outputs": ["overall", "dimensions", "evidence", "confidence"],
  "comparison": { "mode": "absolute", "eligible_group_id": null },
  "apply_review_decisions": false,
  "apply_edits": false
}
~~~

Output per image:
~~~json
{
  "media_id": 123,
  "image_revision": 7,
  "image_sha256": "actual-current-sha256",
  "aesthetic_profile": "pixel-art@1",
  "overall_aesthetic": 8.5,
  "dimension_scores": {
    "composition": 8.2,
    "color_and_light": 8.7,
    "artistic_execution": 8.4,
    "completeness": 8.0,
    "expressive_impact": 8.6
  },
  "observations": {
    "strengths": ["readable silhouette and deliberate palette"],
    "weaknesses": ["minor unintended pixel clustering"],
    "visual_distractions": []
  },
  "confidence": "medium",
  "training_suitability": null,
  "review_decision": null
}
~~~

Numbers/observations above are **illustrative placeholders**, not image-specific factual findings.

### 10.5 Standalone vs composite behavior (proposed)

- **Single node**: `aesthetic.evaluate` can evaluate one media ID or a collection range. The daemon creates durable work and Agents pull leases. A score-only run **does not** create a caption, modify image/crop, alter normalized tags, set Keep/Reject/Defer, delete, or export.
- **Full curation**: when the job explicitly selects both aesthetic assessment and caption/tags, the *same Agent vision pass* may produce them together; the daemon persists separate artifacts and validates each target. A failure in one artifact must not imply success in another.
- **Relative preference**: separately request a pair or eligible interchangeable difference-set ranking, with a human/source-confirmed comparison group. The output is a non-destructive ranking recommendation; no auto-collapse of same-character alternative artworks.
- **Human review**: if `confidence=low` or style is ambiguous, surface rationale and `needs_review`. Do not treat low confidence as low aesthetic quality.
- **Post-edit**: crop/restore changes current image revision; old report remains historical, and a new current-image report requires a new full-frame inspection if requested.

### 10.6 v1 acceptance and performance requirements (proposed)

- Input/output schemas and CLI/MCP versioning match generic `node.run` and `work.submit` contracts.
- Score-only path demonstrably has **zero** mutation of `media.keep`, deletion queues, media bytes, `.tags.txt`, `.raw_tags.txt` or `.txt`.
- Cross-version and cross-style results are not mixed by default; missing/ambiguous Profile is visible, not hidden.
- Batch leases and per-image atomic commits follow ADR-002, with stale SHA, expiry and duplicate idempotency tests.
- Aesthetic work queues use Agent capacity budgets, not long held database/media locks. Compare single-pass combined scoring+caption to separate calls on fixed fixtures, and track human disagreement and uncertain rates.
- Run edge-case test sets for intentional minimalism, painterly brushwork, limited-palette pixel art, Galgame scene framing, text/watermarks, unusual perspectives, and low-resolution high-quality work.
- No automatic rejection, deletion or representative policy is enabled merely by configuring a Profile.

**Still open for further owner review:** exact Profile prompt/rubric contents, whether to support custom Profile authoring in v1, how to collect human calibration labels, and whether aesthetic assessments should offer a separate *non-binding* curation recommendation.

**Integration note (ADR-005):** W16 / `aesthetic.evaluate` remains score-only and does not directly change deletion reviews. In the **composite illustration-curation workflow**, a separate `content.classify` -> `review.route` path **adds confirmed imported comics to the existing pending deletion-review collection**, with current SHA, reason and evidence. No immediate file deletion; user confirmation follows the existing GUI flow. Pre-download comic-tag exclusions have no managed media ID and therefore cannot be queued as library deletions. See [ADR-005](COMIC_PAGE_EXCLUSION_AND_OPTIONAL_LABELING.md).

## 11. Deferred comic/manga aesthetic assessment (accepted scope limit)

**Decision:** Comic/manga aesthetics are **not** an initial W16 supported evaluation Profile. A manga/comic page often depends on narrative continuity, panel transitions, visual hierarchy across pages, lettering and pacing; a single-image generic illustration score would incorrectly reward or penalize editorial choices. Avoid creating or improvising a scoring rubric until a separate, calibrated multi-panel/page-context flow is explicitly approved.

### 11.1 Applicability rules (v1 proposal for implementing the accepted exclusion)

- Known comic pages, manga story pages, multi-panel strips, sequential storyboards or related narrative-page images: `aesthetic.evaluate` returns `skipped` with `reason_code: "UNSUPPORTED_AESTHETIC_FORMAT"`, `aesthetic_score: null` and `format_hint: "comic_or_manga"`. **Skipped is not low quality, bad training data, or rejected.**
- For style-only manga/anime-looking **single standalone illustration**, it is permissible to select the general-illustration Profile **when the asset is actually a standalone illustration**, not a story page. Do not assume any black-and-white drawing is a manga page.
- When image content is ambiguous, return `needs_classification` / `deferred` without inventing an aesthetic score; optional human classification can unlock the appropriate existing non-comic Profile. Do not silently force the generic Profile onto uncertain manga pages.
- The format gate can use source metadata and available visual observations; source tags alone are fallible. It must not invoke an extra hosted vision call merely to resolve exclusion without explicit scope/consent.
- This exclusion is **only** for the aesthetic evaluation workflow W16/N23. It must not automatically prevent collection metadata, technically supported imports, hash/integrity checks, user-requested caption/tag extraction, or independent duplicate analysis. Existing media-type restrictions (e.g. animated assets) are unchanged.
- Do **not** add manga content to automatic aesthetic rankings, representative selection, Keep/Reject, pruning, export eligibility or deletion based on a missing aesthetic score. Human decisions stay authoritative.
- If W16 is one step inside a larger workflow, record `skipped` for W16 on manga pages and allow other **independently eligible** nodes to proceed. Do not stop the entire job or count unsupported aesthetics as a stage error.
- Keep future manga scoring as a separate proposed flow requiring explicit context-aware criteria (panel flow, page narrative, lettering and sequential readability), dedicated human reference labels, and a specific owner decision.

### 11.2 Contract example (illustrative only)

~~~json
{
  "media_id": 123,
  "node_type": "aesthetic.evaluate",
  "state": "skipped",
  "reason_code": "UNSUPPORTED_AESTHETIC_FORMAT",
  "format_hint": "comic_or_manga",
  "overall_aesthetic": null,
  "review_decision": null,
  "source_unchanged": true
}
~~~

### 11.3 Acceptance tests

1. A verified manga page receives no aesthetic score, no fallback generic score and no Keep/Reject/deletion side effect.
2. A black-and-white standalone illustration is not auto-excluded solely because of monochrome rendering.
3. An uncertain panel/page detection is marked uncertain/deferred and preserves the source.
4. Full workflow execution correctly counts manga-aesthetic skips while continuing eligible technical/annotation work.
5. No standard Profile comparative ranking includes skipped comic pages as if `null` meant zero.
