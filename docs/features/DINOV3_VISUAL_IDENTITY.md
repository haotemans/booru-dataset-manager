# DINOv3 visual identity and representative selection

## Separate concepts

Do not collapse these concepts into one `duplicate` boolean:

1. **Exact duplicate:** same file bytes, detected by content hash and enforced by database uniqueness policy.
2. **Visual variant:** resized, recompressed, color-adjusted or cropped version of an image.
3. **Same subject/identity:** same character, person, object or visual identity across different scenes/poses.
4. **Similar composition/style:** visually similar but not necessarily the same image or identity.

DINOv3 embeddings are retrieval features, not proof of semantic identity and not an image-quality score by themselves. Candidate relation type must be recorded separately from the similarity score.

## Recommendation and human review

The system can rank candidate representatives, but the user has chosen **vision-API assessment plus human review** rather than trusting unattended aesthetic judgments.

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

- Automatic grouping only uses a versioned threshold policy evaluated on labeled data; do not hard-code an unvalidated threshold.
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
