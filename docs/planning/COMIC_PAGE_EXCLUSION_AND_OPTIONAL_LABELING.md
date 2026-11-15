# ADR-005: Comic-page detection routes managed images to the normal deletion-review collection

**Status:** PARTIALLY ACCEPTED (2026-11-15). Comic/story pages are out of scope for the illustration dataset, and the owner explicitly requires **recognized comics already held in the managed library to enter the existing deletion-review collection**, rather than merely being excluded from exports. Automatic final deletion is **not** implied; it follows the existing GUI confirmation and cancellation workflow. Source-only records with no downloaded media must not masquerade as deletable library media. This is a design-only Draft PR, **not yet implemented**.
**PR:** #11.
**Related:** [ADR-003 workflow/node catalog](AGENT_WORKFLOW_NODE_CATALOG.md), [ADR-004 aesthetics](AGENT_AESTHETIC_EVALUATION_WORKFLOW.md), [existing ingestion](../features/INGEST_PIPELINE.md), [existing Agent review implementation](../../crates/infrastructure/src/agent/state.rs).

## 1. Final intended pipeline semantics

The new illustration-curation pipeline must distinguish *source filtering*, *comic classification*, *review-queue routing*, and *confirmed physical deletion*. They are not interchangeable.

| Where detected | Action | Review/deletion collection? | Physical file deletion |
| --- | --- | --- | --- |
| Source post with exact \`comic\` tag **before download** | Skip acquisition, persist source skip reason \`COMIC_TAG_EXCLUDED\` | **No media deletion entry:** nothing was imported; optional read-only skipped-source history in GUI | None |
| Image **already imported** and source tag \`comic\` recognized | Queue existing media ID for ordinary **pending deletion review** with reason \`comic_tag\` and current SHA | **Yes**, in the same collection/interface as other Agent-rejected images | Only after existing user confirmation |
| Imported image classified as sequential comic by Agent vision | Queue existing media ID for ordinary **pending deletion review** with reason \`comic_visual\`, classification confidence and evidence | **Yes**, normal deletion-review collection | Only after existing user confirmation |
| Downloaded but **not yet imported** temporary file classified as comic | Do not import. Drop verified temporary bytes safely and persist source exclusion record | No managed media ID; no fake deletion candidate | Delete temporary uncommitted file only |
| Uncertain page / uncertain single illustration | Hold out of automatic illustration export, retry or optional review; do **not** queue deletion as confirmed comic | Not a comic-reject entry | None |
| Standalone illustration in manga/anime style, manga cover, single-page illustration | Eligible for normal curation | No comic-based deletion | None |

**Scope:** narrative comic/manga story pages, 4-koma, sequential strips and storyboards; not monochrome art, a single caption bubble, an anime/manga look or a specific artist. Danbooru general tag \`comic\` is an inexpensive early hint but is neither complete nor infallible; run an existing Agent vision pass for untagged images when selected by the workflow.

## 2. Existing deletion-review implementation is the compatibility target

Existing MCP \`bdm_assess\` with \`decision=reject\` saves a review in \`agent_reviews\`, reports \`queued_for_deletion=true\`, and does **not** delete files. GUI snapshot uses \`decision='reject'\` plus current image SHA to list review items. Final UI confirmation uses \`delete_review_scope\`, revalidates current SHA and optionally resolves duplicate survivor associations. GUI cancellation calls \`clear_agent_rejections\` and persists a \`defer\` decision.

**New comic path must reuse the same deletion-review collection/UX and final confirmation**, not create a disconnected quarantine page or a parallel deletion subsystem. Proposed review report fields: \`reason_code=COMIC_CONTENT\`, \`reason_detail=comic_tag|comic_visual\`, \`content_form=sequential_comic\`, \`confidence\`, \`evidence\`, \`model_or_policy_version\`, \`media_id\`, \`image_sha256\`, \`workflow_job_id\`, \`actor_id\`, \`created_at\`. Keep the source raw tags intact.

To avoid conflating aesthetics with deletion, the new workflow executes **\`content.classify\`** and **\`review.route\`** as distinct logical nodes. The former returns a typed classification. The latter, when part of the owner's requested illustration-curation workflow and authorized to write, routes eligible comic verdicts into the normal review collection. **W16 score-only aesthetics remains non-destructive and never enqueues rejection as a side effect.**

Do not reuse \`bdm_assess\` with a fabricated low aesthetic score to force \`reject\`: comic exclusion is an **out-of-scope content decision**, not an aesthetic failure.

**Already reviewed by a human:** do not overwrite a persistent Keep/cancel/manual override with a new automated comic verdict. Expose a conflict/needs-review event if the policy requires revisiting it. Retries are idempotent; only one active pending review per image revision and reason, no duplicate cards.

## 3. End-to-end workflows and single-node behavior

### W02 source discovery / pre-download gate

1. Prefer supported provider query \`-comic\` where verified, but also inspect exact whitespace-delimited \`tag_string_general\` (or complete raw tags when appropriately categorized) for \`comic\`; record skipped-source ID and reason.
2. If the post is not downloaded, there is no \`media_id\`: do not fabricate an entry in the **media deletion queue**. Optional GUI source history may show excluded posts without a Delete action.
3. For uncategorized/untagged posts, normal static-media download/verification proceeds according to scope. A separate **strict preview mode** may classify before import where preview/vision access and consent are available, at additional cost.

### W14 illustration curation / existing-library cleanup

1. Cheap exact source tag check and prior valid content-classification cache.
2. For media that already reaches Agent visual reasoning, include \`content_form=standalone_illustration|sequential_comic|uncertain\` and confidence/evidence in the **same visual analysis pass** as caption/aesthetic when possible; no second model call solely for this classification.
3. On confirmed \`sequential_comic\`, call \`review.route\` with \`decision=reject\`, \`reason_code=COMIC_CONTENT\`, current media revision, idempotency key and lease ownership. Save the pending review into the **existing review list**, not into a separate export-only filter.
4. Suppress downstream aesthetic/caption/illustration export work for the rejected image within the active workflow. Do not overwrite caption/tags/source files to force this state.
5. GUI displays the ordinary pending-review card, reason, source of verdict and original/current preview. User may confirm or cancel; both are audited and preserved across restart.
6. On actual user confirmation, recheck current image SHA, shared collection references, file ownership and review state before physical deletion. Follow the existing deletion service and recovery journal.

### Standalone nodes

- \`node.run content.classify\` by default performs **classification only**, so an Agent can inspect an image without unexpected changes.
- \`node.run review.route\` takes an explicit reviewed verdict and authorized route policy; it can **only queue pending review**, never perform physical deletion.
- \`workflow.run --profile illustration-curation\` explicitly composes classification -> conditional \`review.route\` for confirmed comics -> skip remaining image-specific illustration nodes. The workflow's declared side effects must appear in \`workflow.plan\`.
- \`node.run aesthetic.evaluate\` on comic content returns \`skipped / UNSUPPORTED_AESTHETIC_FORMAT\` and does **not** itself enqueue a rejection; callers who want deletion review must use the routing node/composite workflow.

### Proposed structured result (illustrative, not implemented)

~~~json
{
  "media_id": 123,
  "expected_sha256": "actual-current-image-sha",
  "content_form": "sequential_comic",
  "confidence": "high",
  "evidence": ["multiple ordered narrative panels"],
  "route": {
    "collection": "existing_deletion_review",
    "status": "pending_review",
    "reason_code": "COMIC_CONTENT",
    "source": "agent_visual"
  },
  "files_deleted": 0
}
~~~

## 4. Performance, scheduling, accounting

- Exact \`comic\` tagging in remote metadata is checked **before original download** to avoid network costs when possible. If no media exists, there can be no deletion of library bytes and no normal media review item.
- Image-form classification piggybacks on **Agent visual curation**. Do not require an extra model round trip where the image is already presented. A download-only workflow can choose fast tag-only or strict visual-preview screening, with explicit cost/consent.
- No routine manual comic pre-screening. Human involvement remains the **existing final deletion approval**, not an extra classifier phase.
- Per-item review routing is atomic, protected by the current image SHA, lease fencing and idempotency key. A GUI manual cancel or Keep has higher priority than an old Agent result.
- Record \`source_comic_skipped\`, \`media_comic_pending_review\`, \`media_comic_review_cancelled\`, \`media_comic_deleted_by_confirmation\`, \`media_comic_uncertain\` separately; never report pending as physically deleted.
- Source exclusions are not failed downloads; comic review/reject is not the same as duplicate review or low aesthetic score.
- Existing source originals and human changes are preserved until confirmed deletion. Follow multi-collection association integrity rules; never blindly delete a shared asset solely because one illustration workflow excludes it.

## 5. Tests before merging implementation

1. \`comic\` source tag on never-downloaded post -> skipped source, **not** fake media review entry.
2. Imported post with \`comic\` tag -> appears in the **existing GUI deletion-review collection** with source, reason and current hash; file still exists.
3. Imported untagged comic recognized by Agent -> same existing deletion-review collection; caption/aesthetic/illustration export skipped for that item.
4. Manga-style illustration, cover and monochrome artwork -> not rejected as comic merely by style.
5. Uncertain classification -> held out of export but not added as confirmed comic rejection.
6. Repeated Agent submission, lease expiry and stale SHA -> no duplicate card or stale overwrite.
7. Human cancellation/Keep -> persists and wins over retry, restart and rescan.
8. Confirmed final deletion -> operates only after explicit existing review confirmation and proper shared-reference checks; cancel deletes no bytes.
9. W16 score-only aesthetic -> no \`agent_reviews\` reject write or \`media.keep\` side effect.
10. Source/managed review progress counts and audit match real outcomes across daemon restart.

## 6. Optional future comic-specific labeling (out of scope)

A specialized future dataset may store page-level panel boxes, reading order, dialogue bubbles/legible text, per-panel captions/tags, cross-panel continuity and story-flow assessment. **Do not implement a manga aesthetic Profile or comic-only OCR/segmentation pipeline in PR #11**. Focus on correct exclusion + ordinary deletion-review routing for current illustration curation.

**Open detail:** If the owner wants every excluded *remote source post* visibly listed alongside deletion candidates, implement a clearly labeled non-deletable source-skip history section; do not create a false managed-media deletion record.
