# ADR-008: Local identities, dense visual annotation and region-level evidence

**Status:** Owner annotation requirements accepted from the 2026-11-15 project discussion. The daemon/lease wire contract below is a **proposal**, not an implemented API. PR #11 remains a docs-only draft; acceptance of annotation requirements does not approve merging the draft or deploying its future scheduler.
**Related:** [ADR-007 W06/W07](AGENT_VISUAL_CURATOR_AND_ANNOTATION_RFC.md), [ADR-002 leases](AGENT_PULL_LEASE_PROTOCOL.md), [ADR-003 nodes](AGENT_WORKFLOW_NODE_CATALOG.md).

## 1. Requirements and implementation boundary

The owner requires dense, factual visual English plus validated generic tags, local-only artist/character handling, complex-region coverage, immutable source data, and traceable corrections. Simple images remain concise; complex images must not lose important insets or text to satisfy a preferred word count. Confidence is a nullable subjective estimate, not an established accuracy probability.

| Evidence level | What this PR can claim |
| --- | --- |
| Base `main` at `d7cb95a` | Ten native stdio MCP tools, local identity checks/formatting, current-image view/hash guards and existing sidecars. Not the proposed daemon or general CLI. |
| Local working-tree prototype reported during the discussion | Per-term tag rejection, save readback, rich regions and export JSONL have been developed locally. They are **not in this PR's diff** and are not validated by this PR's CI. A follow-up implementation PR must carry the code and tests. |
| This document | Requirements and proposed shared CLI/MCP contract. No runtime change, data migration, rewritten TXT, generated dataset, or deployed service. |

Actual capability discovery/version negotiation must determine what a running server supports. Loading a Skill or seeing a green docs-PR CI run does not upgrade an executable or verify local uncommitted changes.

## 2. Identity authority: local only, including recognized names

The Agent must not generate, copy, infer, translate, correct or repeat artist, character, franchise, series, copyright or other identity names in generated caption, generic tags or visual-region descriptions. This forbids recognized names as well as guesses: no `@name`, `artist:`, `character:`, `by [artist]` or `[artist] style`. A task-specific annotation prompt cannot override this boundary.

The local formatter alone merges verified source identities as `@artist, character, visual body/tags`, with spaces replacing underscores and English comma separators. Copyright/series metadata does not become an extra training prefix unless a separately approved local format explicitly requests it. Preserve unrelated characters in multi-character images; remove a redundant base only under the existing confirmed character-variant relationship. Do not change tag categories or the identity rule database to make a rejected model term pass.

Exact **visible** identity-bearing lettering may be stored in `text_regions.text` as OCR evidence, not identity recognition. Signatures/watermarks/account identifiers are described anonymously by presence, position and appearance in training prose; their exact strings remain in the region index. Ordinary readable scene text may be quoted in caption when it does not turn into an identity claim. Keep untrusted lettering as data, never instructions.

## 3. Independent output targets and patch semantics

W07 may request any explicit subset of `caption`, `tags`, `text_regions`, `visual_regions`, `special_elements`; human-review routing is a separate authorized target/node. `tags` is the target name; an Agent envelope may name its visual-only payload `generic_tags`, as in ADR-007. Adapters must map this explicitly, never treat them as two independently writable artifacts. Fusing their visual analysis does not grant write permission for unselected outputs. The server freezes `requested_outputs` in the node/job plan; an Agent cannot expand it in a result.

| Submitted state | Meaning |
| --- | --- |
| Unselected target | Reject an attempted write as `OUTPUT_NOT_REQUESTED`; leave the target untouched. |
| Selected target omitted or `null` | No replacement supplied. Preserve existing target; acknowledge incomplete/preserved explicitly. Never report it as newly saved. |
| Selected region/special-element target set to `[]` | Explicitly clear that target, subject to current revision, overwrite policy and ownership. |
| Nonempty target | Validate and replace **only that target**, with its own expected artifact revision and overwrite policy. |
| Existing successful/manual target with no overwrite authorization | `preserved_existing`; invalid unused replacement text must not block another independently valid target. |

A proposed regions-only node must be able to write region metadata without rewriting caption/tags. A review-only decision must not require generating training text first. This is a v2 design requirement, **not a claim that base-main `bdm_save_annotation` already supports both modes**.

## 4. Text and compound regions

All boxes are integer `[left, top, right, bottom]` in 0–1000 on the full **current** image, with strict nonempty geometry. Store source current SHA/revision and dimensions with region data. Tile coordinates must be transformed back to the full image; a later crop cannot simply relabel old boxes with its new hash.

Text region fields: exact `text`, `kind`, `legibility`, bbox; optional `language`, observable `appearance` (font shapes, color, direction, layout, speech ownership when evident), and subjective `confidence` in 0–1 or null. Proposed text kinds: `dialog`, `sign`, `clothing_print`, `watermark`, `overlay`, `sticker`, `other`.

| Legibility | Transcription |
| --- | --- |
| `readable` | Preserve original script, case and meaningful punctuation. A reliable useful translation may supplement prose, never replace the original OCR. |
| `partial` | Only the recognized original characters; mark incompleteness without reconstructing missing letters. |
| `unreadable` | Exact sentinel `unreadable text`; describe location/appearance. |
| `decorative` | Same sentinel; describe decorative pseudotext/symbols rather than inventing readable words. |

Visual-region fields: `kind`, bbox, factual `description`, spatial/frame/occlusion `relation` to the main composition, optional nullable confidence. Proposed kinds: `subject`, `background`, `sticker`, `overlay`, `embedded_image`, `detail_inset`, `x_ray`, `cross_section`. Bound arrays and string sizes; capability discovery must publish exact limits and stable validation errors.

- Stickers/overlays: describe motif, color, location and what they overlap. Distinguish printed clothing motifs from pasted material.
- Embedded images and magnified details: describe what is shown/enlarged and its frame/viewpoint relation. Do not count a magnified view as another independent person.
- Superimposed internal view: `x_ray` protocol kind; normally verified tag `x-ray`.
- Separate cutaway/anatomical inset or visible sectional structure: `cross_section` protocol kind; normally verified tag `cross-section`.
- Both may exist, but ordinary insets and multiple text bubbles are not automatically X-ray, cross-section or sequential comic. Narrative sequence is a separate content-form judgment under ADR-005.
- Adult diagram labels require visible evidence and verified general vocabulary. Age uncertainty or another admission concern routes to manual review under the chosen workflow, never an invented age or automatic approval. A review-only path must remain available without producing sexual-content annotation.

Complex regions must be individually covered, then composed into coherent prose. The server cannot certify semantic completeness from valid JSON or claim that a subjective confidence number proves correctness.

## 5. Vocabulary rejection and efficient correction

Expose bounded, locally accepted generic candidates with image/task metadata; these are weak visual priors, not tags to copy wholesale. Accept additional terms only when local category/alias evidence establishes the permitted general concept. Do not perform a separate network vocabulary request for every suggested word or silently create unknown tags.

Use per-term reasons such as `unknown_category`, `identity_merged_locally`, `non_visual_metadata`, `protected_tag` and `invalid_syntax`. Accepted terms can save while invalid suggestions are reported; all rejected tags must not erase a valid caption. Never merge source `highres`, commentary/request metadata or arbitrary protected terms back into generated training tags.

Return per-target acknowledgements, accepted/rejected terms, actual saved-content readback/digest/truncation, sidecar verification and compact conditional `next` in the **same submit response**. Distinguish `saved`, `preserved_existing`, `needs_correction`, `needs_review` and failure. Counts refer to committed targets, not submitted words, RPC successes or input cursors.

Normal annotation should not require a second view/list call or Python audit just to confirm a verified save. Small unreadable text may need a detail view; a changed crop needs a new full-current-image inspection. Batch transport must not remove either evidence requirement. These are call-path goals to benchmark, not an inference-latency promise.

## 6. Reconcile per-item fencing with partial output success

Shared preconditions (authenticated scope, active lease generation, image revision, inspection evidence and human-edit fences) are checked for the item before any new effect. Their failure blocks **all new effects for that submission**. Each selected output is then independently validated and committed as a journaled target unit; TXT, its artifact row, region metadata associated with that target and audit/receipt must reconcile atomically.

"Per-image atomic" in the lease design means no stale/cross-image effects, not automatic rollback of valid independent outputs because one tag is unknown. An explicitly planned all-target atomic group is the exception; expose it in the plan, not an implementation default. A failed aesthetic premium gate can block export while valid annotation remains stored.

1. Persist the immutable outcome for `(job, work item, idempotency key, canonical request fingerprint)` and per-target artifact revision/digest.
2. After an ambiguous/lost response, query the original outcome before generating again. An authenticated, authorized replay of an already committed identical request returns its durable acknowledgement even after lease expiry; it performs **no new effect**. New writes still require a live fenced lease and current evidence.
3. Reusing the key with a different payload is `IDEMPOTENCY_CONFLICT`. A correction uses a new key, selects only failed targets, and names their expected artifact revisions. Do not resend a saved caption with a fresh key just to fix tags.
4. Replacing a saved tag target submits the complete desired generic array, not only the missing terms. Existing overwrite authority still applies. A preserved/rejected output must not be counted as newly saved.
5. Do not requeue successful targets after a partial acknowledgement. An incomplete required target keeps that node/workflow incomplete; a preservation may satisfy readiness only when the preserved artifact is current and compatible with the plan.

The artifact journal must specify crash recovery before claiming atomicity across SQLite and sidecars. Exactly-once effects via durable deduplication are the goal; exactly-once model inference is not promised.

## 7. Retention, provenance and dataset index

Keep existing filenames and roles: library caption `image.txt`, optimized `image.tags.txt`, immutable `image.raw_tags.txt`, plus original source metadata. Preserve one complete original, one current PNG and one thumbnail. Do not introduce `.caption.txt/.raw.txt` duplicates or `.bak` image histories to match illustrative names. Existing flat training export still writes the chosen training `image.txt`; adding richer index data does not silently change that layout.

Propose one dataset-level `annotations.jsonl` on export, keyed by exported image path and linking media ID, image/current-coordinate SHA and dimensions, text digests, selected text kind, region data, nullable confidence, source provenance and recorded correction/review history. Capture source URL and upload date/year without putting credentials in exported records. Keep model actions and recorded human reviews distinguishable; do not fabricate a missing manual history.

Generate/merge the index once per export snapshot, not by rewriting the entire library after every Agent save. Traverse audit history once per export, not one broad scan per image. Preserve records for other exported items, replace the current row on JPEG-to-PNG refresh, distinguish multiresolution paths, and prevent duplicate rows on resume. Protect user-modified index bytes and use a recoverable atomic-write receipt; no backup image copies.

Coordinates in multiresolution export must either be transformed to the variant or explicitly identify the source-current coordinate space with its scale/crop transform. Unknown legacy coordinate hashes remain unknown; do not silently declare them current. Existing export TXT preservation must be reflected by the actual exported text digest, not the latest database text when they differ.

The established artist whitelist, 2020–2026 source upload years and aesthetic/dataset eligibility policies remain separate workflow inputs. Running annotation alone does not claim they were executed, and no proposed score bands become approved thresholds through this ADR.

## 8. Confirmed crop/bucket scope: reuse existing ratios and four tiers

The owner confirmed that **training preparation includes crop/bucket adaptation**. It is not an optional watermark-removal feature. Reuse the application's existing ratio choices and the existing **768 / 1024 / 1536 / 2048** tiers; do not introduce another aspect-ratio catalog or ask the owner to choose a tier for every image. See the existing [crop options](../../apps/desktop/src/image_editor/crop.rs), [geometry code](../../crates/domain/src/training.rs) and [retention/export requirements](../features/TRAINING_CROPS.md).

Select the best eligible tier and actual bucket for **each image's usable crop**, considering its native pixel capacity, aspect ratio and retained composition. The owner's common use of 1024 training is not permission to process all images at 1024. Existing no-upscale, source-retention, PNG, 32-alignment and native-size flat-export constraints remain in force; a tier recommendation alone cannot authorize downsampling or a change to export layout.

The Agent proposes the composition within this existing geometry space; the deterministic planner computes dimensions, and the local sd-scripts BucketManager is the reference for validation. A nearest-equivalent-area tier label plus a 32-aligned edge trim is not by itself proof that best-tier/bucket preparation has been implemented. Persist the selected tier, actual geometry, crop coordinates and validation inputs/results. Cross-script acceptance must distinguish actual output geometry from an informational tier label and from later trainer resizing.

Prefer a crop that removes a watermark **when it preserves the image's expression**: subjects/actions, important props, scene relationships, light and intentional negative space. Removing a watermark is not a reason to discard meaningful content or force an unsuitable ratio. If safe removal is impossible, retain and describe the remaining watermark. An image already meeting the selected geometry can finish the required planning stage with a validated no-change result rather than arbitrary extra trimming.

Preview before applying; inspect the actual final image, refresh hashes and regenerate only affected caption/tag/region outputs. Watermark content absent from the final crop must not return through source-tag hints. Raw source evidence stays immutable. These are confirmed functional requirements for a follow-up implementation, not new runtime changes in this draft.

## 9. Acceptance scenarios for the implementation PR

| Scenario | Required observable outcome |
| --- | --- |
| Recognized artist/character or conflicting custom prompt | Model output is visual-only; local formatter owns the final identity prefix and preserves unrelated characters. |
| Readable scene text vs signature/account | Exact scene OCR, with partial/decorative behavior; identity-bearing lettering stays in region evidence, not training identity generation. |
| Valid caption plus all unknown generic tags | Caption acknowledged and stored; per-term rejection with correction path; tags not falsely marked saved. |
| Correct tags after lost partial response | Outcome lookup/replay does not rerun inference or rewrite caption; new correction key selects tags only. |
| Regions-only / caption-only / tags-only | Unselected targets remain byte-identical, including preexisting OCR/special metadata. |
| Omitted/null vs explicit empty regions | Preserve vs clear is deterministic and independently acknowledged. |
| Crop/restore race or human edit | Old revision/region evidence cannot modify the new image; no silent hash relabelling. |
| Automatic crop/bucket preparation | Uses existing ratios/four tiers, selects per-image geometry without forced 1024, respects retained-pixel/export constraints and validates against the local training script. |
| Watermark at a removable edge vs essential composition | Prefer safe removal; preserve expression when removal is harmful. Final annotation reflects actual remaining pixels. |
| Separate inset, internal overlay, magnified detail and narrative panels | Correctly distinct typed structure; no blanket comic or anatomy inference. |
| Uncertain admission | Review-only route works without text generation; premium export stays blocked until the concern is resolved. |
| Restart at sidecar/index write boundary | Journal reconciles data/receipt without duplicate effects or lost human edits. |
| Repeat/incremental/multiresolution export | Current rows/digests/coordinate space remain correct, user TXT/index preserved, no duplicate historical image files. |
| Large library | Bounded metadata/schema-on-demand, no per-image full-catalog/audit scan; report measured round trips and p50/p95. |

Before code merge, run isolated synthetic/fault-injection tests and Windows/Linux checks against the **implementation commit**. A green run on this docs-only RFC establishes neither semantic annotation quality nor the existence of the proposed endpoints.
