# ADR-007 / W06+W07: Training suitability, visual assessment and one-pass Caption/Tag annotation

**Status:** DESIGN DRAFT / OPEN FOR OWNER REVIEW (2026-11-15). W05 difference preservation was accepted before this draft; the **W06 quality policy and W07 annotation output/commit contract are not yet frozen**. PR #11 remains design-only; no runtime code or data has been changed.
**Related:** [ADR-001 persistent daemon](AGENT_DAEMON_AND_INTERFACE_CONTRACT.md), [ADR-002 Agent-pull leases](AGENT_PULL_LEASE_PROTOCOL.md), [ADR-003 nodes/workflow](AGENT_WORKFLOW_NODE_CATALOG.md), [ADR-004 aesthetics](AGENT_AESTHETIC_EVALUATION_WORKFLOW.md), [ADR-005 comic review routing](COMIC_PAGE_EXCLUSION_AND_OPTIONAL_LABELING.md), [ADR-006 dedup](AGENT_DUPLICATE_WORKFLOW_RFC.md), [current caption/tag rules](../features/CAPTIONS_AND_TAGS.md).

## 1. Distinct responsibilities and a shared Agent visual pass

These must be separately *requested and stored*, even when derived in one model call:

| Capability / logical node | Objective | Executor | Output | Side effects |
| --- | --- | --- | --- | --- |
| `media.technical_check` | file integrity, source hash, dimensions, format/animation, no-upsizing and basic image decoding checks | local Rust | pass/fail/unknown reasons | no model calls; may mark technical eligibility |
| `content.classify` | distinguish sequential manga/comic pages from standalone artworks | cheap source metadata then Agent vision when already requested | content_form, confidence, reasons | composite illustration workflow routes confirmed *managed* comics to existing pending-deletion queue; standalone classification read-only |
| `aesthetic.evaluate` (W16) | assess artwork visual appeal in versioned style Profile | Agent | separate score/rubric/confidence/evidence | score-only, no implicit Keep/Reject |
| `training.evaluate` (W06) | assess usefulness *for explicit training target* and distinctness/coverage | Rust target policy + Agent visual judgement as needed | training score + reasons + target profile | decision suggestion only unless composite workflow explicitly includes review route |
| `caption.generate` (W07) | English factual visible-image caption | Agent, source identities merged locally | generated caption artifact | only targeted caption write |
| `tags.generate_or_optimize` (W07) | Danbooru-style generic tags grounded in pixels, preserving source identity tags | Agent + local vocabulary and identity validator | generated normalized general tags, rejected suggestions | only targeted tags write, never raw source |
| `text_regions.detect` (W07) | bounding boxes for legible text and distinct graphical overlays | Agent | optional structured region list | bounded structured metadata, independent from caption |
| `review.route` | queue well-founded reject/defer/keep outcome in existing review UI | local policy/authorized Agent result | current-review state with reason | never immediate physical delete |

**Important:** `training_suitability` is not `aesthetic`, and a high aesthetic score is not proof an image improves any target training set. Dedup candidates should be evaluated before promoting "redundancy" into a low training-value reason. Confidence is separate from score; uncertain is not zero.

`vision.curate` is a **fused execution shape**, not a mandate to write every output. It can ask the same Agent to produce content form + aesthetic report + target suitability + caption + generic tags + regions + crop *proposal*. Each output has its own requested flag, validation, artifact version, error and write permission. Score-only, captions-only, tags-only, or regions-only node jobs must leave all unselected targets untouched.

## 2. Suggested W06: technical and training-quality flow

1. Scope: media IDs/collection snapshot, source category, training target (e.g. illustration, pixel-art or character/style LoRA), aesthetic Profile, optional quality policy version and required output types.
2. **Local preflight** before Agent: valid current image SHA, source availability, dimensions, no source corruption, content-type gate, cached technical check. A hard technical failure is recorded as such; do not infer that it is a low aesthetic score.
3. **Content form**: if confirmed comic page in a composed illustration workflow, enqueue existing deletion review according to ADR-005, then skip image-specific aesthetic/caption/training export. For a standalone score-only job the node does not change review state.
4. **Aesthetic report**: only if selected and not a comic; use versioned Profile. Null/low confidence is permitted.
5. **Training suitability**: target-specific:
   - legibility, visible subject completeness, photographic/illustrative consistency to requested training task;
   - caption/conditioning coverage that can be stated without guessing;
   - use dedup/dataset coverage evidence when available rather than visually claiming uniqueness;
   - image defects relevant to training such as corrupt regions, severe compression, unintended overlays, confusing collage;
   - do not rank by resolution alone, artistic complexity alone, presumed artist popularity or source rating.
6. **Decision suggestion** `keep / defer / reject_for_review`, structured reason(s), confidence and whether an in-place crop might help. A score-only W06 remains non-destructive; a composed quality-screen flow may explicitly include `review.route` to queue rejecting images in existing GUI collection, followed by **human final confirmation**.
7. Preserve prior valid human Keep/cancel and reviewed artifacts; do not overwrite decisions with a lower-confidence automated result. A changed SHA/profile/model must invalidate only affected derived records.
8. **No speculative "score<7=reject" policy**. Thresholds, high-confidence reject categories, minimum training eligibility and human override behavior must be calibrated and explicitly approved. If no validated policy exists, store scores/recommendations but do not manufacture a rejection.

### W06 proposed typed result

~~~json
{
  "media_id": 123,
  "image_revision": 7,
  "content_form": "standalone_illustration",
  "technical": {
    "state": "pass",
    "minimum_dimension_ok": true,
    "integrity_ok": true
  },
  "aesthetic": {
    "profile": "general-illustration@1",
    "overall": 8.6,
    "confidence": "medium"
  },
  "training": {
    "target_profile": "illustration-curation@1",
    "suitability": 8.1,
    "confidence": "medium",
    "reasons": ["complete readable subject", "no obvious unintended visual artifacts"]
  },
  "review_suggestion": "keep",
  "review_reason_code": null,
  "crop_proposal": null,
  "write_review_state": false
}
~~~

Numbers above are examples, not results for any actual image. The illustrative `write_review_state=false` means **no Keep/Reject/Defer mutation**, despite the suggestion.

## 3. Suggested W07: Caption, Tags and text-region flow

**Current contract:** three UTF-8 per-image sidecars: `image.raw_tags.txt` (immutable source), `image.tags.txt` (normalized/optimized tags), `image.txt` (factual natural-language caption). Verified artist and character name fields are merged locally. Existing image-based Tag writes verify suggested generic tags against the local category-0 vocabulary; source-protected categories are not model-authoritative. A model can suggest novel visual terms but unverified new terms cannot bypass local validation. Export mode chooses appropriate content; do not overload raw source.

### Agent caption expectations

- English; observe image independently even when a batch is sent. Image pixels are primary; source tags are **fallible hints**. Never blend attributes across images.
- Describe visible subject count, placement, hairstyle and hair color, eye color if actually discernible, expression and gaze, body/head orientation, pose and hand/object interaction, clothes/layers/texture, accessories, scene/background, important objects, framing/camera angle, color and light, stylistic technique, meaningful fine detail.
- Use evidence-grounded specificity, not unsupported story, identity or artist guesses. Do not describe occluded anatomy. Avoid repetitive filler, boilerplate, speculative mood and "high-quality masterpiece" style tags as factual content.
- Retain source-confirmed artist/character identity only through local formatter; generated caption body may use visible generic descriptions but must not invent proper names. Keep training caption distinct from the aesthetic explanation.
- In a batch, each item output is mapped by immutable media ID and expected SHA/revision. A missing or failed item must not borrow another item's caption.
- Captions should be proportionate to visible complexity; max byte/token limits are model/provider-specific and validated. Existing Agent MCP caption limit is currently 8192 chars; do not silently break compatibility.

### Tag expectations

- Preserve raw source exactly and keep artist/character/copyright identity categories source-owned; handle verified aliases and existing manual overrides with local rules.
- Optimize/generalize only ordinary/general tags grounded by actual pixels and the validated local tag vocabulary. Convert underscore style according to the established target format, without silently changing category meaning.
- If the image visually contradicts a nonprotected hint, correct/drop the unsupported generic hint when authorized. Do not erase source tags; store rejected/unverified candidate tags with reason if useful for audit.
- Tag generation and normalization are separate output targets; no automatic change to the human-curated identity/tag rules.
- Caption-only and tags-only modes must be individually executable; either may share visual reasoning with W06/W16 when the combined workflow explicitly requests it.

### Text regions / image artifacts

- Optional text boxes on current image, each with type (speech/text overlay/watermark/sign/diagram/etc), normalized `bbox_1000_xyxy`, textual content only if legible, and reading confidence. Distinguish intended artwork lettering from harmful overlays; no blanket aesthetic penalty.
- Bounding boxes must lie inside 0..1000 with valid geometry and current revision; descriptive special-element boxes should be separately typed when required.
- **No OCR dependency or extra OCR pass by default**. The Agent can describe visible text during the existing model call; if text is unclear, record illegible rather than hallucinate. An optional dedicated OCR node would be a separate future flow.

## 4. Fused Agent interface: one reasoned pass, per-target commits

### Image view / task issue

Daemon prepares only compact media manifests (ID, SHA, dimensions, source generic hints, protected-identity references, selected Profile, target outputs and current annotation states). A lease claims multiple images but images should be displayed to Agent in a *separately controlled* pixel budget: a suggested starting point is 4-8 metadata items, then 1-4 full-frame images per vision call, validated by benchmark. A contact sheet is triage, not enough to authorize image mutations.

### Result envelope (illustrative, not implemented)

~~~json
{
  "job_id": "job-curate-72",
  "lease_id": "opaque-lease",
  "generation": 12,
  "items": [
    {
      "work_id": "curate:123:7",
      "media_id": 123,
      "expected_sha256": "actual-current-SHA",
      "idempotency_key": "curate:123:7:run-1",
      "requested_outputs": ["content_form", "aesthetic", "training", "caption", "tags", "text_regions"],
      "content_form": "standalone_illustration",
      "aesthetic": {
        "profile": "general-illustration@1",
        "overall": 8.6,
        "confidence": "medium"
      },
      "training": {
        "target_profile": "illustration-curation@1",
        "suitability": 8.1,
        "confidence": "medium"
      },
      "caption": "A character in a pale outfit stands against a softly lit background.",
      "generic_tags": ["1girl", "long hair", "standing"],
      "text_regions": [],
      "crop_proposal": null,
      "review_suggestion": "keep",
      "reason": "Grounded visible details and suitable composition"
    }
  ]
}
~~~

**Validation/commit rules:**
- An Agent **can reason about multiple outputs in one visual call**, but the daemon verifies each schema and permissions separately and only commits requested targets to a still-current revision. A composite per-image result has per-target acknowledgment: `accepted`, `preserved_existing`, `needs_review`, `rejected_invalid`, or `skipped`. A failed tag validation must not falsely mark caption/aesthetic failed, unless the workflow explicitly requires an all-target atomic output.
- Use short media-revision compare-and-swap and a journal for SQLite + sidecar writes. Never hold media/global DB lock across model reasoning, asset view, OCR/codec/preview or network. Do not replace an existing human-reviewed source or target text from an unrelated job.
- Reusing valid current artifacts is conditional on image SHA, selected output/version, source and prompt/model fingerprint, human review state. No repeated model call for already-complete, still-valid unchanged outputs.
- If the image needs a crop, the first pass **only proposes** it. The crop path requires explicit preview/authorization and returns a new image SHA; final-image annotation is committed only after viewing the new current image and verifying the edited composition. Avoid generating two full captions unless crop actually changes training content.
- If content form is confidently `sequential_comic`, do not create general-illustration aesthetic/caption/training outputs in the composed illustration workflow. Route **already imported** comics to the existing pending-deletion queue through a separate authorized `review.route` node; never delete bytes automatically.
- A model error, refusal, truncated output, missing image ID, incompatible Profile, rate limit, lease expiry or revision conflict gives typed partial failure and bounded retries. Do not clobber last valid artifacts.

## 5. CLI / MCP design surface

| Action | CLI proposal | MCP proposal | Notes |
| --- | --- | --- | --- |
| Just inspect eligibility | `bdm node run media.technical_check --input @-` | `bdm_node_run` | local, no Agent |
| Only aesthetics | `bdm node run aesthetic.evaluate --input @-` | `bdm_node_run` | score-only, separate W16 report |
| Only training suitability | `bdm node run training.evaluate --input @-` | `bdm_node_run` | context/target-specific; non-destructive by default |
| Only caption | `bdm node run caption.generate --input @-` | `bdm_node_run` | one image or batch; protected identity merge |
| Only tags | `bdm node run tags.generate_or_optimize --input @-` | `bdm_node_run` | vocabulary validation, raw tags preserved |
| Combined W06/W07 | `bdm workflow run --profile visual-curation --input @-` | `bdm_workflow_run` | one Agent visual pass per eligible image where feasible |
| Agent work | `bdm work lease/view/submit` | `bdm_work_lease`, `bdm_work_view`, `bdm_work_submit` | typed output-selective payload and item IDs |
| Route review | `bdm node run review.route --input @-` | `bdm_node_run` | only explicit authorized composite workflow queues rejection |

An Agent can request only a **node** or a **workflow**, and one workflow may conditionally include W05 dedup, W16 aesthetics, W08 crop, W06 screening, W07 generation or W11 export. The daemon must show planned side effects before execution. The client need not load every tool schema into its model context; capability discovery and compact generic command tools suffice.

## 6. Resource scheduling and measurements

- Local media preflight and source identity normalization execute without an Agent lease. Agent-pull work waits in the durable queue. Full current-image viewing is required before visually grounded mutation.
- Combine content-form detection, subjective scores, caption, tags and region output when requested; **do not force full flow on caption-only/quality-only calls**. Use output-specific views to minimize model context and I/O.
- Separate image count, preview pixels, encoded bytes, model context budget and Agent lease quota. Use a bounded cache of current thumbnails and reuse source file SHA/model data. No unbounded batch image base64 text.
- Score/cache reuse keyed to current image SHA + Profile/rubric + model/prompt/policy versions and selected outputs. Invalidate crop-dependent caption on crop, and keep original source raw tags and historical reviews.
- Scheduler never assumes control over an external Agent's provider inference concurrency. Its local controls are leases, memory/IO CPU pools and resource backpressure. Respect provider consent for sending image pixels to an external model.
- Benchmark p50/p95 of preflight, Agent wait, image transfer, end-to-end accepted captions, tags accepted/rejected, aesthetic ranking consistency and false-positive review routing against a representative manually checked sample.

## 7. Critical discussion decisions (not yet accepted)

1. **W06 keep/defer/reject routing:** should training-quality Agent judgments automatically **queue** low-suitability/clearly defective images into the existing deletion-review collection (with human final confirmation), or should W06 only generate scores and route rejections solely on an explicit separate run? Do not conflate this with score-only W16.
2. **Caption detail:** concise factual description vs **dense training description**; whether to use one adaptive prompt based on image complexity, with maximum length and no hallucinated details.
3. **Target-specific suitability:** should default target be general 2D illustration style or user-specified dataset Profile for each collection? Collection profile must freeze across the job.
4. **Annotation overwrite:** default preserve existing reviewed caption/tags, explicit overwrite only on a named output target vs always replace unreviewed artifact on a newly requested optimization run.
5. **Selected output failure semantics:** per-target independent commit (recommended, matches current sidecar behavior), with a workflow that can require an all-target validation gate before export.
6. **Uncertainty:** define benchmarked thresholds for missing detail, unclear text, small/occluded eye color, ambiguous artistic style, and training suitability; do not guess without sample labels.

**No new CLI/MCP/node API is implemented by this RFC.** No background model work, physical deletion or data migration is authorized by discussing these contracts.
