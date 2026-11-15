# ADR-004: Aesthetic evaluation as a first-class independent Agent workflow

**Status:** PROPOSED FOR DISCUSSION. The owner explicitly requested that aesthetic judgment be covered as its own flow. Numeric rubric/profile defaults, tools and decisions below are not yet accepted.
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

## 9. Decisions still to settle with owner

1. Aesthetic rubric and default profile: universal balanced baseline vs explicit user style preferences vs collection-specific profile.
2. Assessment behavior: score-only as default (recommended); whether to optionally emit non-binding keep/defer/review recommendations.
3. Confidence and fairness: when to request a second review or human confirmation, and when to refuse to compare dissimilar styles.
4. Full workflow combination: fuse aesthetics with one vision-pass annotation by default if selected; can be disabled independently.
5. Dataset curation thresholds and profile-specific representative ranking policy; never assume one cutoff applies across all art.
