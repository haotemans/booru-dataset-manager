# ADR-007 / W06+W07: Training suitability, visual assessment and one-pass Caption/Tag annotation

**Status:** PARTIALLY ACCEPTED (2026-11-15). Owner selected **W06 policy A: clearly unsuitable high-confidence images automatically enter existing pending-deletion review**. **New accepted priority: aesthetics is a HARD quality gate for the premium illustration dataset**: artwork that clearly falls below an approved style-aware aesthetic standard can be unsuitable even if it is technically valid, and may be queued for review. Weak/uncertain borderline art is deferred and excluded from the premium export, not automatically retained. Final physical deletion remains human-confirmed. The detailed W06 rubric/threshold calibration and W07 annotation schema/defaults below are still proposed for review. PR #11 remains docs-only; no runtime code or data has changed.
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
6. **Decision suggestion** `keep / defer / reject_for_review`, structured reason(s), confidence and whether an in-place crop might help. **Accepted W06 A:** in a user-authorized *quality-screen workflow*, only `reject_for_review` outcomes satisfying versioned high-confidence unsuitability gates are routed into the **existing pending-deletion collection**; all other cases are kept or deferred. The **standalone `training.evaluate` node remains score-only**, unless the requested workflow explicitly includes `review.route`. Human final confirmation is mandatory.
7. Preserve prior valid human Keep/cancel and reviewed artifacts; do not overwrite decisions with a lower-confidence automated result. A changed SHA/profile/model must invalidate only affected derived records.
8. **No speculative `score < 7 = reject` rule.** Accepted policy A is a **categorical, evidence-backed high-confidence eligibility gate**, not a low aesthetic score cutoff. Concrete defect categories and thresholds must be tested/calibrated. Without a validated high-confidence criterion, return `defer` or `keep` and never fabricate an auto-reject.

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

1. **W06 routing: policy A ACCEPTED.** High-confidence, clearly unsuitable images automatically enter existing pending-deletion review **only within an explicitly authorized quality-screen workflow**. **Confirmed poor aesthetics is an eligible rejection reason** in a quality-screen workflow when style-correct, evidence-backed and benchmarked; uncertainty alone never routes rejection. Standalone scoring remains non-destructive. Remaining decisions: tested eligibility reason codes, recoverability and confidence calibration.
2. **Caption detail:** concise factual description vs **dense training description**; whether to use one adaptive prompt based on image complexity, with maximum length and no hallucinated details.
3. **Target-specific suitability:** should default target be general 2D illustration style or user-specified dataset Profile for each collection? Collection profile must freeze across the job.
4. **Annotation overwrite:** default preserve existing reviewed caption/tags, explicit overwrite only on a named output target vs always replace unreviewed artifact on a newly requested optimization run.
5. **Selected output failure semantics:** per-target independent commit (recommended, matches current sidecar behavior), with a workflow that can require an all-target validation gate before export.
6. **Uncertainty:** define benchmarked thresholds for missing detail, unclear text, small/occluded eye color, ambiguous artistic style, and training suitability; do not guess without sample labels.

**No new CLI/MCP/node API is implemented by this RFC.** No background model work, physical deletion or data migration is authorized by discussing these contracts.

## 8. W06 quality policy A — accepted routing, proposed concrete evaluation contract

### 8.1 Separate technical health, aesthetics and training suitability

A result is not a scalar "image quality". The daemon stores **independent channels**:

| Channel | Interpretation | Who supplies it | Action |
| --- | --- | --- | --- |
| `technical` | format/decoder/checksum/size/crop feasibility | Rust local validators | fail/valid/unknown; deterministic preflight |
| `aesthetic` | artistic appeal *in a style-specific versioned Profile* | Agent W16 | W16 standalone is score-only; **composed W06 uses approved style-aware aesthetics as a hard premium-dataset eligibility gate** |
| `training` | suitability *for a declared dataset and goal* | Agent + local target-specific criteria | 0-10 or null, reasons, confidence |
| `content_form` | standalone illustration vs sequential comic vs uncertain | source tags + optional Agent view | confirmed managed comics follow ADR-005, independent of scores |
| `duplicate_relation` | whether other images make this instance redundant | W05 pHash/DINO + group review | only W05 selects group survivor and queues nonwinners |

**The target Profile is required and frozen in a job snapshot.** Examples: `general-illustration@1`, `pixel-art@1`, `character-lora@1`, or `style-lora@1`. Different targets can disagree about suitability without changing a global "bad image" flag. The same library image may be useful in a different permitted workflow, so review routing must carry target/scope and respect shared references.

### 8.2 Proposed rejection-eligibility matrix

| Signal / example | Evidence | Proposed W06 response | Reason code |
| --- | --- | --- | --- |
| Invalid/corrupt original/current bytes | verified local format/decoder/checksum failure, after bounded recovery attempt | `defer_for_recovery` or `reject_for_review` only if verified irrecoverable for target | `UNREADABLE_OR_CORRUPT` |
| Image below **actual** configured target usability constraints | measured native pixels + named training geometry/profile, no upscale path | `reject_for_review` only if not recoverable by a valid crop/variant and policy has a verified hard floor | `BELOW_TARGET_REQUIREMENT` |
| Wrong content domain (e.g. screenshot/photo in strictly 2D illustration dataset) | source + full-image evidence, clearly outside frozen target definition | `reject_for_review` when high confidence; otherwise defer | `TARGET_MISMATCH` |
| Severe unintentional artifact covering essential visible content | full view/necessary detail tile; substantial watermark, compression corruption, pasted overlay | suggest `repair_or_crop` when safe; queue only if not usable or repairable under target policy | `UNRECOVERABLE_VISUAL_DEFECT` |
| Confirmed narrative comic page | exact source tags or visual observation under ADR-005 | route via **comic classification path**, not W06 low score | `COMIC_CONTENT` |
| Verified duplicate group nonwinner | W05 group winner and immutable revision evidence | route via **W05**, not W06 low training score | `DUPLICATE_GROUP_NON_SURVIVOR` |
| Confirmed poor aesthetics against approved style-aware premium-data bar | complete-image view, rubric/Profile, grounded weak composition/light/color/artistic coherence observations, calibrated high confidence | **`reject_for_review` eligible** in explicitly authorized W06 quality-screen flow, otherwise defer when uncertain | `AESTHETIC_BELOW_STANDARD` |
| Intentional blur, rough sketch, flat colors, low-res pixel style, stylized anatomy, text integral to artwork | style intent and available context | **do not penalize by generic illustration criterion**; use appropriate Profile | `STYLISTIC_INTENT` |
| Mild noise, small watermark, limited detail, imperfect background | scene-dependent and potentially repairable | `keep` or `defer` / conditional repair | `MINOR_OR_FIXABLE` |
| Evidence unclear, model disagreeing or incompatible scores | incomplete current view, low confidence, ambiguity | `defer`; do not enter confirmed-delete queue | `INSUFFICIENT_EVIDENCE` |

**Resolution policy:** the current Rust training module has four target tiers `768/1024/1536/2048` with step `32` and a native crop acceptance check of short side >=576 and area >=550000 pixels. These numbers are **different concepts**: tiers are export/training geometry choices and the current native crop minimum is not a new blanket "all <768 pictures are bad" aesthetic rule. A target dataset may adopt a stricter, explicitly frozen hard minimum, but W06 must check the relevant real feasibility, not compare a filename or nearest tier. No blind upscaling.

### 8.3 Three actionable outcomes plus a recoverability detour

| Outcome | When | Effect in complete W06 quality-screen workflow |
| --- | --- | --- |
| `keep` | passes both training-target requirements **and approved minimum aesthetic quality** for premium illustration curation | continue requested W07 annotation or next configured node |
| `repair_or_crop` | image has a correctable defect; original retained | optionally schedule **explicit** W08 preview/reinspection; otherwise `defer`, not delete |
| `defer` | evidence incomplete, score uncertain, potentially valuable outlier, model unavailable | persist reason and next action; no deletion queue; may retry or review later |
| `reject_for_review` | clearly **unfit for the chosen training target**, high confidence, supported by approved condition and current revision, no untried reasonable remedy | enqueue **existing GUI pending deletion review** with target, reason, evidence and current SHA; final physical deletion requires user batch confirmation |

**Mandatory auto-route gate** (logical AND):
1. It is a workflow with explicit `quality_screen_route_rejects` permission; a standalone W06 node is score-only.
2. The image/form/target Profile is known, image evidence/technical evidence is current, and the classification is not uncertain.
3. At least one **approved, versioned reject-eligible reason code** is true, with verifiable evidence and high calibrated confidence, or a deterministic terminal technical failure with completed recovery policy.
4. **Aesthetic below standard is allowed as an approved rejection reason**, but only when the style-appropriate premium-quality rubric was applied to the actual full image, specific visual shortcomings are recorded and confidence/calibration requirements pass. Do **not** reject merely for sparse details, monochrome art, pixel style, intentional roughness, unconventional style, unverified duplicate status or a resolution bucket label.
5. There is no unresolved meaningful recovery/crop path and no conflicting prior human Keep/cancel or manual protected state.
6. Versioned per-media current SHA/revision, Agent lease/fencing and user permission all pass the commit transaction. Otherwise **defer** or return conflict, never silently queue.

**Confidence:** `high/medium/low` is a reporting label, *not* sufficient by itself. A high confidence requires specific image-grounded observations or deterministic measurements and passing tests on labeled edge cases. Numeric aesthetic gates require **versioned per-style calibration**; never treat an uncalibrated raw model 7.0 as a universal truth. Before calibration, borderline/low-confidence images are held out of premium export and deferred, while clearly evidenced poor-quality cases can be proposed for review only under separately validated rules. Keep a separate `decision_confidence` and per-criterion `evidence_source`.

### 8.4 W06 state transitions and durable review

    input_media
      -> technical_preflight
      -> content_form_gate
      -> Agent visual assessment (only if requested/needed)
      -> keep | defer | repair_candidate | reject_for_review
      -> [if reject] review.route -> pending_deletion_review -> human confirm/cancel
      -> [if keep] selected W07/W08/export prerequisites

- Agent result and `review.route` can be planned as one logical composite workflow, but the **score report remains separate** from the authorized pending-review state. Rejects do not make `media.keep=false` or physically delete any bytes.
- Once `reject_for_review` queues a media ID, the default complete curation flow should **skip W07 and W16 for that image** (unless an explicitly requested independent job overrides that task scope). Preserve all earlier valid annotation artifacts.
- A subsequent change in image revision, target policy, content-form classification or manually protected decision invalidates stale review proposals and must be reconciled before final confirmation.
- Review queue cards show current preview, reject reason, evidence/source, confidence, affected target, potential repair, actor/model version, timestamp, and `files_deleted=0`. If this image also has comic or duplicate reasons, aggregate reasons without replacing a human cancellation; current `agent_reviews` one-review-per-media representation may need schema/workflow migration.
- An image that is genuinely reusable elsewhere should not be globally deleted merely because it fails **one** scoped training target; on physical deletion approval, validate shared collection/source references and permit removing just the unsuitable dataset association when applicable.
- State/audit events: `w06.keep`, `w06.defer`, `w06.repair_candidate`, `w06.pending_deletion`, `w06.review_cancelled`, `w06.user_confirmed_deleted`; each tied to job, media ID, SHA, target policy and idempotency key.

### 8.5 W06 score rubric — advisory, not a rejection formula

Score `training.suitability` from 0..10 (null if no reliable basis), alongside typed dimensions:
- `subject_readability`: are needed subjects/poses visible, count and occlusion accurately representable?
- `composition_for_target`: usefulness of framing and spatial relationships for this particular target (aesthetic beauty is separate).
- `visual_integrity`: severe unintended artifacts/overlays vs intentional stylization.
- `conditioning_feasibility`: can one grounded caption and tags describe the current image without unsupported guesses?
- `dataset_contribution`: novelty/diversity derived from W05 and catalog coverage, not inferred solely from an isolated picture.

Suggested *interpretation anchors for TRAINING suitability* (not aesthetics): `0..2` unusable, `3..4` major reservations, `5..6` usable with compromises, `7..8` strong fit, `9..10` excellent fit. The **aesthetic** field has its own versioned premium-quality threshold and confidence gate (section 12); do not conflate the two scales. Review routing is controlled by an approved reason code and evidence, not by aesthetic score alone. An output may be `overall=null` while still reporting deterministic technical state. Always persist score Profile, model/prompt version, confidence and short evidence.

## 9. W07 precise one-pass labeling pipeline (proposal)

### 9.1 Node processing order

1. **Scope and target selection**: `caption`, `tags`, `text_regions`, optional `aesthetic`/`training`/`content_form` and `crop_proposal` are **explicit output flags**. A caption-only task has no side effects on tags/review state.
2. **Reuse and source protection**: read existing current generated/reviewed artifacts and immutable source `raw_tags.txt`, source-verified artist/character/copyright identities, language/config/Profile. Skip target artifacts when valid/unchanged unless a new explicit optimization job requests replacement.
3. **Inspect current full frame** via `work.view` and optionally view region tiles only for small text/complex garment details. Source tags are fallible hints; the image is authoritative for visible details. Protect identity locally.
4. **One model reasoning pass** for all requested eligible outputs: structured JSON result with visual Caption, generic tag candidates, text regions and optional scores/crop plan. The client Agent does the inference; the daemon does not call a provider autonomously.
5. **Image-form and W06 gate** before writing model-generated target annotations in a combined illustration workflow. Confirmed comics and valid high-confidence W06 rejects go to existing review queue; skip target caption/tag writes. Uncertain images do not generate fabricated content.
6. **Crop detour, only if needed and selected**: proposal -> exact preview receipt -> authorized apply -> **new image SHA** -> full-current re-view -> re-evaluate/adjust final-image captions/regions before commit. Do not overwrite images on the basis of a model's crop proposal alone.
7. **Per-target local validation**: caption consistency/length/protected identities, approved generic vocabulary for tags, 0..1000 region geometry/legibility and bounded counts; reject or defer uncertain assertions, record unverified tags and reasons.
8. **Commit**: per-image current-revision CAS with separate artifact outcomes and short journaled sidecar/SQLite write. Return `saved`, `preserved_existing`, `rejected_invalid`, `needs_review`, `skipped` for each selected output.
9. **Audit/reuse**: persist SHA, source refs, model/prompt/version, selected fields, tag validation failures, receipt IDs, result hashes, next stage. Never overwrite original raw tags or unrelated manually reviewed artifacts without a corresponding explicit authorized target replacement.

### 9.2 Caption contract and training utility

Recommend `detailed-grounded-en@1`: detailed *English* image-based training caption with comma-separated visual clauses, proportional to scene complexity, **not** a free-form artistic critique or invented storyline.

Order as useful, without rigid repetitive templates:
- subject count and relative placement; each character's visible hair/eyes/expression/gaze/head orientation and distinguishing visible features;
- pose/body orientation, hand/object interaction, clothing layers/cuts/colors/folds/accessories, shoes/headwear;
- nearby objects, background, setting, composition/camera/crop/perspective and foreground/midground/background relationships;
- color palette, lighting direction and quality, shadows, texture, visible linework/paint/render style, important scene detail;
- accurately located text/bubble/sticker/special element where applicable, with separate typed BBox rather than forcing coordinates into prose.

Do **not** invent artist/character proper names, occluded anatomy, hidden objects, illegible words, timestamps or narrative context. Distinguish visible detail from unreliable source tags. If eye color is not discernible, omit it. Do not copy the full input Danbooru tag list into sentences. No cross-image contamination within a batch.

**Source identity merge:** the local formatter adds trustworthy artist names with the configured prefix (default `@artist`) and confirmed character names as plain identifiers *before* the generated visual description, separated by comma-space. The Agent output contains visible generic facts only. Preserve raw source ordering and underscores in `image.raw_tags.txt`. The generated English caption uses spaces and punctuation as chosen by the caption Profile.

**Bounds:** existing MCP caption limit is 8192 characters, so retain that as a hard compatibility maximum pending schema v2. A flexible target should depend on actual scene complexity and model context/pixel budget. A caption that is 2000 words of boilerplate is not automatically better than one that fully covers meaningful visible details. Selectable compact vs detailed modes are possible but not yet approved.

### 9.3 Tag contract

- Agent returns *generic visual tags only*. Source-verified artist/character/copyright categories never come from model guesswork and stay protected. Danbooru general hints may be corrected or omitted if visually false, without touching raw source.
- Use established normalized tag output in `image.tags.txt`, no leading identity contamination. A locally verified category-0 vocabulary is currently enforced by the MCP implementation. New unsupported generic tag guesses are rejected or flagged, not blindly accepted.
- Keep source tagging semantics: e.g. visible color, clothing, pose, setting, expression, object interaction, composition and stylization only if actually visible. Avoid subjective quality tags, speculative names and scene inference.
- Respect verified character implication logic: don't erase a specific variant in favor of a generic base character; multiple visible variants may coexist. Keep manual metadata and known aliases.
- Existing MCP upper bound of 512 proposed input tags / 256 chars each remains a guardrail; **do not** aim to emit that many tags. Validate canonicalization, unknown terms and duplicates.
- Tag-only mode writes only the tags target. Caption-only mode writes only the caption target. A bad Tag result should not erase a good current caption.

### 9.4 Region/BBox contract

- Model supplies normalized `bbox_1000_xyxy` = [x1,y1,x2,y2] on the **current full-frame revision**, with 0<=x1<x2<=1000 and 0<=y1<y2<=1000. Coordinates use image boundaries, not thumbnail viewport boundaries.
- Type examples: `text_overlay`, `speech_bubble`, `sign`, `watermark`, `sound_effect`, `diagram`, `sticker`. Location, legibility and typed field are stored independently from caption/tags.
- Text content is null or omitted if illegible; do not hallucinate OCR. No separate OCR model call by default. One image may have multiple regions, capped by configured resource limits.
- After crop, BBoxes must be transformed/verified against the new current image dimensions and revision; no stale regions from the old frame may be considered current.

## 10. Shared Agent result and scheduling details (proposal)

### Request profile

~~~json
{
  "workflow": "visual-curation",
  "version": 1,
  "scope": {"collection": "artist-or-collection-id"},
  "target_profile": "general-illustration@1",
  "quality_policy": {
    "mode": "route_high_confidence_unfit",
    "review_destination": "existing_pending_deletion",
    "allow_physical_delete": false
  },
  "requested_outputs": [
    "content_form", "training", "aesthetic", "caption", "tags", "text_regions"
  ],
  "write_policy": {"preserve_manual": true, "raw_tags_immutable": true},
  "image_transfer_consent": "explicit"
}
~~~

`workflow.run` displays planned side effects (including potential review queue inserts) and requests required scope/consent before execution. Standalone `node.run training.evaluate` remains score-only even when a review hint is returned.

### Agent payload

~~~json
{
  "work_id": "curate:123:7",
  "media_id": 123,
  "expected_sha256": "current-verified-sha",
  "lease_generation": 12,
  "content_form": "standalone_illustration",
  "training": {
    "target_profile": "general-illustration@1",
    "overall": 3.4,
    "confidence": "high",
    "reason_codes": ["UNRECOVERABLE_VISUAL_DEFECT"],
    "evidence": ["A large unintended overlay obscures the only main subject"],
    "recoverability": "none_known"
  },
  "aesthetic": {
    "profile": "general-illustration@1",
    "overall": 7.2,
    "confidence": "medium"
  },
  "review_suggestion": "reject_for_review",
  "requested_outputs": ["content_form", "training", "aesthetic", "caption", "tags"],
  "caption": null,
  "generic_tags": null,
  "text_regions": [],
  "crop_proposal": null
}
~~~

Numeric values and evidence are synthetic examples only. This payload is an **Agent proposal**; the daemon independently validates whether the reject reason is allowlisted/verified before queueing.

### Server acknowledgement

~~~json
{
  "media_id": 123,
  "state": "pending_deletion_review",
  "reason_code": "UNRECOVERABLE_VISUAL_DEFECT",
  "quality_saved": true,
  "caption_state": "skipped_due_to_quality_gate",
  "tags_state": "skipped_due_to_quality_gate",
  "files_deleted": 0,
  "review_destination": "existing_pending_deletion"
}
~~~

Each W06/W07 media result is idempotent, with a current SHA and lease fencing. Grouped reviews (W05) and comic review (ADR-005) can coexist with quality reasons in the same existing pending review UI. If other contexts require this image, final user confirmation must handle shared references safely.

**Execution optimization:**
- Local technical checks run while other Agent visual batches execute. The Agent receives a small bounded work lease; batch metadata does not include raw image base64.
- Fusing `content_form + aesthetic + training + caption/tags/regions` in **one visual pass** is preferred when all selected, but an early "clearly reject" can skip emitting long captions/huge tag arrays. This is one requested model response, not a guarantee of lower token usage; benchmark overall accepted images/min and provider cost.
- Caches per image current SHA + source/tag rules + target Profile + quality/aesthetic rubric + model/prompt + requested outputs. A human revision or crop invalidates only dependent results; do not re-download unchanged sources.
- Per-target validation permits independent accepted outputs. A workflow export gate checks all specifically required results are complete/current before export; a tag failure does not silently mark caption as accepted.
- Error responses distinguish `INSPECTION_REQUIRED`, `UNSUPPORTED_FORMAT`, `REASON_NOT_APPROVED`, `NOT_REPAIRABLE_UNVERIFIED`, `REVISION_CONFLICT`, `LEASE_EXPIRED`, `PERMISSION_DENIED`, `RATE_LIMITED`, `TAG_VOCAB_MISSING` and `PARTIAL_RESULT`.
- Measure automatic-review **precision** in human-approved samples (low false-positive reject is critical), deferred percentage, recovered images, residual bad-image export percentage, caption hallucination/identity contamination, locally rejected tags, model calls per image, image bytes, p50/p95 stage latency and peak RAM. Record separately from W16 subjective aesthetic scores.
- Fault tests: CLI session loss after score submission; reject routing retried with same key; human Keep/cancel races Agent outcome; edited image SHA between view and submit; corrupt-source recovery; style-specific pixel art blur false positive; raw tag invariance; multi-image cross-contamination; partial target commit and SQLite/TXT crash journal recovery.

## 11. Remaining decisions for discussion

**Accepted now:** W06 quality-first policy A: high-confidence unsuitable images, including **confidently poor aesthetics under an approved style-aware premium dataset standard**, enter the existing pending-deletion review in an explicitly selected quality-screen workflow. Borderline/uncertain art is deferred and withheld from premium export, not assumed good. Human final batch approval, immutable raw source, and manual overrides remain protected.

**Still open (proposal rather than acceptance):** 
1. Which exact W06 reason-code thresholds qualify for high-confidence routing in v1, **including `AESTHETIC_BELOW_STANDARD`**, and what per-style calibration/evidence is mandatory for each?
2. Caption Profile default: `detailed-grounded-en@1` versus compact vs user-selectable; desired length is derived from scene complexity, not a constant count.
3. When all selected outputs are requested, should an image with `defer` still receive partial annotation, or wait for a retry/review? Recommendation: preserve existing data and do not automatically export until required checks pass.
4. Whether quality-scope rejection means “remove from one dataset” versus “propose physical deletion from managed library”; keep physical deletion explicitly review-gated, respecting shared references.

## 12. ACCEPTED direction: aesthetics-first PREMIUM dataset gate (numerical details proposed)

### 12.1 Product decision and task separation

The owner prioritizes **absorbing strong artwork, not retaining weak artwork merely because it is technically trainable or unusually rare**. For the *premium illustration curation workflow*, **aesthetic merit is a required eligibility gate**, not a decorative score displayed after all images are labeled.

The division of responsibilities remains:
- `W16 / node aesthetic.evaluate` = **score-only**, no hidden Keep/Reject mutation, so independently running an assessment does not unexpectedly alter a collection.
- `W06 / visual quality-screen workflow` = reads a comparable W16 result or evaluates aesthetics in the same Agent pass, applies a **user-approved premium aesthetic quality policy**, and routes confirmed weak artwork to **the existing pending deletion-review collection**. A human batch-confirms actual deletion.
- `W07` = detailed captions/tags only for images that pass the active workflow's quality/content/dedup gates; standalone W07 still works for explicitly targeted media, respecting permissions and user intent.
- `W05` = choose the aesthetically best among interchangeable true duplicates, **but a group winner is not automatically good enough for premium training**. After W05, W06 can still reject the winner if it fails the absolute per-style aesthetic standard; no forced "keep one bad image" exception.
- Technical invalidity, manga exclusion and duplicates keep their own reason codes. Aesthetics should be recorded as **why** an image is unsuitable, not disguised as corrupt or duplicated.

### 12.2 Calibrated policy shape (proposal; NOT approved thresholds)

An example *strict premium* policy for the 0–10 aesthetics scale **after calibration with user-rated samples**:

| Illustrative calibrated aesthetic range | Intended W06 disposition | Export eligibility |
| --- | --- | --- |
| `>=8.0` | `premium_pass` if technically/training eligible | eligible for requested W07/full premium export |
| `7.0..7.99` | `borderline`: conditional Agent comparison/re-review or user-specified keep | **hold out of default premium export** until accepted |
| `<7.0` | `aesthetic_reject_candidate` when style-correct evidence + confidence high; otherwise defer | not eligible; queue existing pending-delete review only if accepted criteria pass |
| `null`, unsupported/ambiguous Profile, model uncertainty | `defer` | not eligible until reliable evaluation |

**These numbers are provisional candidates, not universal model facts or an enabled production policy.** Each Profile (general illustration, pixel art, watercolor, Galgame CG) requires human-rated anchors and a versioned calibration mapping. A local ranking/paired preference result can resolve borderline cases where raw absolute scores drift. User may select different bars by collection/profile; the immutable job snapshot freezes the chosen bar to avoid mid-job inconsistent curation.

**Do not adopt naive top-X-percent filtering without a minimum aesthetic bar.** Selecting the "best" of a poor batch may still leave bad images; selecting only high scores from a narrowly sampled artist may destroy desired variety. Diversity only matters **after** the style-aware quality floor is met. A deliberate high-quality minimalist composition can pass despite fewer objects, fewer details or lower pixel count.

### 12.3 Hard rejection evidence requirements (proposal)

For `AESTHETIC_BELOW_STANDARD`, require:
1. A valid and current full-image view receipt, current SHA, agreed aesthetic Profile/rubric/model prompt and non-comic content eligibility.
2. Explicit visual grounds (e.g. incoherent visual hierarchy, poorly integrated foreground/background, inconsistent unintentional rendering, weak color/light interaction, visibly unfinished execution for that style); no invented artist/style attribution.
3. Calibrated confidence or comparable multi-image reference evidence demonstrating the work falls below the **selected style's** premium-quality floor; uncertain style or disagreement -> `defer` / extra check rather than deletion review.
4. Check whether an intentional style, comic exclusion, meaningful variant, or *reasonable* native crop changes the assessment. Where a feasible correction could materially improve the image, route `repair_candidate` first, then reassess on the edited revision before rejection.
5. User Keep/cancel, reviewed manual content, current SHA and shared library references are protected. Do not delete bytes directly or override a human decision.

Suggested structured report:
~~~json
{
  "media_id": 123,
  "quality_policy": "premium-aesthetic@1",
  "aesthetic": {
    "profile": "general-illustration@1",
    "overall": 6.2,
    "confidence": "high",
    "evidence": ["weak visual hierarchy", "unintended inconsistent lighting"]
  },
  "training": {
    "suitability": 7.8,
    "target_profile": "illustration-curation@1"
  },
  "reason_codes": ["AESTHETIC_BELOW_STANDARD"],
  "decision": "reject_for_review",
  "route": "existing_pending_deletion",
  "files_deleted": 0
}
~~~

Numbers and observations are fabricated **examples of schema**, not scores for an actual image. The example intentionally illustrates that technically trainable (7.8) is not sufficient when aesthetic merit is poor (6.2).

### 12.4 Efficient execution: quality before expensive captions

    verified media + current SHA
      -> cheap local integrity/content/duplicate preflight
      -> Agent full-image aesthetics + training gate (same pass when useful)
      -> premium_pass: emit requested high-detail Caption/Tags/BBox + persist
      -> borderline: defer/compare; no premature export or mandatory huge caption
      -> aesthetics_below_bar with approved evidence: review.route to existing pending-delete
      -> correctable composition: W08 crop preview/apply + re-view/re-score new SHA

One Agent vision request may use **conditional outputs**: high-confidence pass produces rich W07 labels, confident reject produces short scores/reasons only. This is not a guarantee that the model can perfectly branch or never waste tokens; measure accepted captions/min, tokens per premium accepted image and false-positive deletion proposals.

When throughput/cost benchmarking supports it, optionally stage **cheap triage -> deeper review only for borderline**; do not trust tiny contact sheets alone to reject complex works. Reuse W16 reports with matching Profile/version and SHA, and reuse local pHash/DINO dedup evidence. Never submit captions or scores from a stale/cropped revision.

### 12.5 Evaluation and release gates

- Build human-approved high/mid/low aesthetic anchor examples **per art style**, plus hard counterexamples: sparse yet beautiful illustrations, limited-palette pixel art, line sketches with intentional roughness, painterly/watercolor texture, cinematic CG, black-and-white standalone images.
- Calibrate scores and paired preference agreement across at least distinct artists and compositions, not just repeats of one artist's output. Track **false-positive aesthetic rejects**, user cancellation rate, premium-export quality distribution, rejection consistency on reruns/model changes and style-specific bias. Exact pass/fail thresholds for calibration require separate owner approval.
- Test low aesthetic/high technical training result, high aesthetic/low training result, uncertain aesthetics, W05 best-of-bad-group, manual Keep, changed image revision, crop/reassessment, duplicate pending review with another reason, and provider/model version changes.
- Ensure score-only W16 is non-destructive, while composed W06 premium routing is explicit and auditable. No physical deletion prior to user batch confirmation, and no hidden overwrite of raw tags or manually authored artifacts.

**Pending decisions:** precise per-style score bands and the initial curated reference set; premium export should block borderline records by default (proposed); optional user override to accept borderline; how to express a style preference Profile without rewarding only one narrow art aesthetic.
