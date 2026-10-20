# DINOv3 visual identity and representative selection

## Separate concepts

Do not collapse these concepts into one `duplicate` boolean:

1. **Exact duplicate:** same file bytes, detected by content hash and enforced by database uniqueness policy.
2. **Visual variant:** resized, recompressed, color-adjusted or cropped version of an image.
3. **Same subject/identity:** same character, person, object or visual identity across different scenes/poses.
4. **Similar composition/style:** visually similar but not necessarily the same image or identity.

DINOv3 embeddings are retrieval features, not proof of semantic identity and not an image-quality score by themselves. Candidate relation type must be recorded separately from the similarity score.

## Recommendation and human review

The system supports both automatic representative selection and manual selection. For source-confirmed difference sets, create the association automatically and select one representative either by the configured quality-ranking policy or by an explicit user choice. Keep the selection reversible and explainable; a manual choice overrides automation. For broader or uncertain visual relationships, DINOv3 supplies candidates and the user confirms the association. Crop suggestions always require explicit human approval.

The review UI should show candidates side by side with full-resolution inspection, zoom, dimensions, file size, source, artist/character links, review status and quality indicators. Actions should include choose representative, keep as separate variant, mark as related-but-distinct, reject a proposed relation, accept/edit/reject a crop suggestion, defer, and undo. A human decision takes precedence over automated suggestions until explicitly reset.

## User-confirmed representative preference

Ranking preference, in priority order:

1. **Aesthetic appeal and completion:** prefer the image that looks more polished and visually complete.
2. **Minimal visual distractions:** prefer a clean composition with fewer distracting overlays or artifacts.
3. **No visible text:** text-free images are preferred when other quality factors are comparable; visible text is a negative signal, not an unconditional rejection.

Use a vision API to assess these subjective signals and provide concise reasons. The assessment is advisory: confidence/uncertainty should be surfaced and the human reviewer makes the final decision. DINOv3 similarity must not be used as a substitute for aesthetic evaluation.

## Crop suggestions are non-destructive derivatives

The vision API may suggest a crop or framing adjustment when it plausibly improves composition without harming the image's aesthetic intent. For example, a portrait image with a 9:16 aspect ratio may look better if the crop window is shifted slightly upward. This is an example, not a fixed rule.

Required behavior:

- Return a crop proposal (normalized rectangle or equivalent coordinates), target aspect ratio if relevant, and a short rationale; do not modify the source file.
- Display original and proposed crop side by side, with an adjustable crop window and a clear indication of what would be excluded.
- The reviewer must explicitly approve, adjust, or reject the crop. Do not auto-apply it to originals or silently replace the representative.
- Store approved crops as derived assets/recipes linked to the original media ID and source hash. Track crop coordinates, target dimensions/aspect ratio, generator/provider/model version, approval state and timestamp.
- Exports use the crop only when the user explicitly enables the approved derived variant. Original export remains available and unchanged.
- If the proposed crop removes important subject details, text that is part of the artwork, or otherwise changes the image's meaning, reject it or request human review.

## Quality signals and vision API

Use the vision API to assess aesthetic/completion quality, visual clutter and visible text, and to explain its recommendation. Technical signals remain useful supporting evidence:

- Decode validity, sharpness/blur, compression artifacts, clipping/overexposure where measurable.
- Native dimensions, aspect ratio and possible upscaling indicators.
- Metadata completeness and source/original status, shown as context rather than as automatic aesthetic priority.

Do not treat larger dimensions as automatically better. Avoid presenting model outputs as objective truth. Keep provider/model/prompt version, response status, rationale and review outcome for audit and evaluation. Before choosing a provider, decide privacy and cost controls; sending images to an external vision API must be opt-in and clearly disclosed.

## Safe automatic policy

- Automatic grouping of DINOv3-inferred near-duplicates only uses a versioned threshold policy evaluated on labeled data; do not hard-code an unvalidated threshold. Trustworthy source-declared difference-set relationships may be grouped automatically with source provenance.
- Automatic grouping is reversible and non-destructive; ambiguous or cross-identity candidates go to review.
- Exact duplicates can be recognized by hash independently of DINOv3.
- Representative selection is a metadata decision. Keep all original files and source records; do not physically delete or silently quarantine non-selected images.
- Crop suggestions are proposals only. No crop changes the original, and no crop is used in export without explicit human approval.
- Exports default to the selected representative when representative-only mode is chosen; allow exporting all variants and optionally approved crop derivatives.
- Human choices must be preserved as explicit overrides with audit history.
- Re-embedding with a new model/preprocessing version must not silently rewrite prior human decisions.

## Data model additions

Suggested `visual_groups` fields: `id`, `group_type`, `created_at`, `policy_version`, `review_state`.

Suggested `visual_group_members` fields: `group_id`, `media_id`, `relation_type`, `similarity_score`, `embedding_model_version`, `decision_source`, `decision_state`, `added_at`, `removed_at`.

Suggested `representative_decisions` fields: `group_id`, `media_id`, `decision_source` (`automatic` or `human`), `policy_version`, `quality_profile_version`, `reason_json`, `created_at`, `supersedes_decision_id`, `is_current`.

Suggested `media_quality_measurements` fields: `media_id`, `measurement_name`, `measurement_value`, `measurement_version`, `measured_at`, `state`, `error_code`.

Suggested `crop_suggestions` fields: `id`, `media_id`, `source_sha256`, `crop_rect_json`, `target_aspect_ratio`, `provider`, `model_version`, `rationale`, `review_state`, `approved_at`, `created_at`. These are logical proposals until migrations are implemented.

Enforce at most one current representative per eligible group transactionally. Keep decision history; do not mutate old decisions in place.

## Evaluation requirements

Evaluate grouping separately from representative ranking and crop proposals. For grouping, report precision/recall, hard-negative false positives, candidate recall at K and latency. For representative ranking, collect human pairwise preferences or group-level preferred-image labels; report top-1/top-k agreement and disagreement rates. For crop proposals, collect accept/edit/reject rates and track whether reviewers believe the crop improves framing without losing important content. Include 9:16 portrait examples with varied subject placement, text, watermarks, cropped and alternate-pose examples. Split evaluation data by identity/group to reduce leakage.

Do not enable unattended representative selection by default until evaluation establishes acceptable performance. Human review remains required for final representative and crop approval.


## Tiered grouping behavior

- **Exact duplicates:** identify with content hashes and database-enforced uniqueness; do not infer byte identity from DINOv3.
- **High-confidence near duplicates:** resized or recompressed versions may be automatically grouped only when a versioned policy has passed evaluation on labeled examples. Decisions must be reversible and preserve all originals and provenance.
- **Semantic or broader visual relations:** different poses/scenes, crops that alter content, alternate frames, different character identities, or merely similar composition/style are surfaced as candidate relations for human review. Similarity alone must not silently assert identity.
- **Uncertain cases:** retain as review candidates. Do not substitute an arbitrary fixed score threshold for evaluation evidence.

Every automated and human decision records relation type and decision source. Human decisions override automation until explicitly changed, with audit history preserved.


## Broad visual-association groups and difference sets

An association group may contain several related but non-equivalent images, including difference/alternate artwork sets such as variants of the same Danbooru work. This is a relationship container, not a duplicate bucket. Each member must carry an explicit relationship type and provenance, for example:

- exact byte duplicate;
- resized/recompressed near-duplicate;
- crop or content variation;
- difference/alternate artwork variant;
- alternate pose/frame;
- same subject or character in another scene;
- same artist, or merely similar composition/style.

Use source-provided relationship metadata when the adapter can reliably obtain it, recording the provider and remote relationship identifier/URL. If source metadata is unavailable or ambiguous, make association a human-confirmed decision; embedding similarity alone is not enough to assert a broad semantic relationship. DINOv3 can retrieve candidates for review but must not silently turn all visually similar images into one identity group.

### Representative selection scope

A broad association group does not automatically have one representative for all members. Representative selection operates only within an explicitly eligible subset of interchangeable near-duplicates/variants for the intended export or review purpose. Difference images that intentionally depict distinct expressions, outfits, censorship states, or other meaningful artwork changes must remain selectable as separate variants. Do not exclude them from export merely because they share an association group. Preserve every original and allow users to inspect, keep separate, split, regroup and undo decisions.


## Difference-set representative selection

For an explicit, trustworthy source-declared difference set, automatically create the association and offer two selection modes:

- **Automatic:** select one representative using the configured quality ranking: aesthetic appeal/completion first, then fewer distracting artifacts, with text-free artwork preferred when other factors are comparable. Record the ranking and policy versions and a concise rationale. If the scoring/vision service cannot provide a usable result, mark the set as needing selection rather than inventing a result.
- **Manual:** show the variants side by side and let the user choose one. A manual decision overrides the automatic choice and is recorded in history.

Representative-only export includes the selected image for each eligible difference set. All variants remain in the managed library, and an explicit all-variants export remains available. Do not physically delete unselected images. This rule applies to source-confirmed difference sets or other groups whose members are eligible substitutes; it does not collapse an entire character, artist, or broad visual-association group to one image.


## Independent analysis lifecycle

DINOv3 analysis is queued by default after successful media import but runs as a separate, persisted work unit. Model/runtime unavailability leaves analysis pending and does not invalidate the import. Embedding or search errors are reported on the analysis job, not by rolling back media. Provide a separate batch analysis/reanalysis action for existing library items and for new model/preprocessing versions. Preserve older analysis outputs and never silently overwrite human-confirmed relation or representative decisions. Display ingest and analysis status independently.
