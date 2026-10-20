# DINOv3 visual identity and representative selection

## Separate concepts

Do not collapse these concepts into one `duplicate` boolean:

1. **Exact duplicate:** same file bytes, detected by content hash and enforced by database uniqueness policy.
2. **Visual variant:** resized, recompressed, color-adjusted or cropped version of an image.
3. **Same subject/identity:** same character, person, object or visual identity across different scenes/poses.
4. **Similar composition/style:** visually similar but not necessarily the same image or identity.

DINOv3 embeddings are retrieval features, not proof of semantic identity and not an image-quality score by themselves. Candidate relation type must be recorded separately from the similarity score.

## Two selection modes

### Automatic recommendation/selection

The system may nominate a representative image for a group and optionally set it as the default export representative. It must not delete, overwrite, or silently quarantine non-representative originals. Store the policy version, candidate set, feature measurements, decision reason, timestamp and whether the result was automatic or human-confirmed. Automatic selection must be reversible.

### Human review

Show candidates side by side with full-resolution inspection, zoom, dimensions, file size, source, artist/character links, review status and quality indicators. Actions: choose representative, keep as separate variant, mark as related-but-distinct, reject a proposed relation, defer, and undo a previous decision. A human override takes precedence over automatic recommendation until explicitly reset.

## User-confirmed representative preference

Ranking preference, in priority order:

1. **Aesthetic appeal and completion:** prefer the image that looks more polished and visually complete.
2. **Minimal visual distractions:** prefer a clean composition with fewer distracting overlays or artifacts.
3. **No visible text:** text-free images are preferred when other quality factors are comparable; visible text should be surfaced as a negative signal, not treated as an unconditional rejection in every context.

These are user preferences to operationalize and evaluate, not claims that the system can already infer aesthetics perfectly. The UI should explain why a candidate was recommended and show uncertain cases for human review.

## Quality signals and ranking

A first implementation should separate user-facing preference signals from technical validity signals.

- **Preference signals:** aesthetic/completion assessment, visual clutter, and visible text presence.
- **Technical support signals:** decode validity, sharpness/blur, compression artifacts, clipping/overexposure where measurable, native dimensions, upscaling indicators, aspect ratio and metadata completeness.
- **Contextual signals:** source/original status can be shown to the reviewer, but must not automatically outrank the user's aesthetic preferences.

Do not treat larger dimensions as automatically better: upscaled images can be inferior. DINOv3 similarity should help decide which images to compare or group, not dominate quality ranking. A visually similar image may have higher quality, but similarity and quality are different axes.

Start with a transparent ranking profile and per-signal explanations. Avoid inventing a universal scalar score before evaluation. If aesthetic/completion and clutter cannot be measured reliably using deterministic signals, use a vision-based assessor only after evaluation against human judgments; otherwise expose the signal for human review rather than pretending it is objective.

## Safe automatic policy

- Automatic grouping only uses a versioned threshold policy evaluated on labeled data; do not hard-code an unvalidated threshold.
- Conservative default: automatic grouping is reversible and non-destructive; ambiguous or cross-identity candidates go to review.
- Exact duplicates can be recognized by hash independently of DINOv3.
- Representative selection is a metadata decision. Keep all original files and source records; do not physically delete or silently quarantine non-selected images.
- Exports default to the selected representative when representative-only mode is chosen; allow exporting all variants.
- A human choice must be preserved as an explicit override with audit history.
- Re-embedding with a new model/preprocessing version must not silently rewrite prior human decisions.

## Data model additions

Suggested `visual_groups` fields: `id`, `group_type`, `created_at`, `policy_version`, `review_state`.

Suggested `visual_group_members` fields: `group_id`, `media_id`, `relation_type`, `similarity_score`, `embedding_model_version`, `decision_source`, `decision_state`, `added_at`, `removed_at`.

Suggested `representative_decisions` fields: `group_id`, `media_id`, `decision_source` (`automatic` or `human`), `policy_version`, `quality_profile_version`, `reason_json`, `created_at`, `supersedes_decision_id`, `is_current`.

Suggested `media_quality_measurements` fields: `media_id`, `measurement_name`, `measurement_value`, `measurement_version`, `measured_at`, `state`, `error_code`.

These are logical schema proposals until migrations are implemented. Enforce at most one current representative per eligible group transactionally. Keep decision history; do not mutate old decisions in place.

## Evaluation requirements

Evaluate grouping separately from representative quality. For grouping, report precision/recall, hard-negative false positives, candidate recall at K and latency. For representative ranking, collect human pairwise preferences or group-level preferred-image labels; report top-1 agreement, top-k agreement and disagreement rates by image type. Specifically label aesthetic/completion, visual clutter, and visible text so that ranking can be audited against the user's priorities. Include upscaled, low-resolution, cropped, watermarked, compressed and alternate-pose examples. Split evaluation data by identity/group to reduce leakage.

Do not enable unattended auto-representative selection as a default until evaluation establishes acceptable performance and the UI makes reversibility clear. Even then, do not delete or silently quarantine originals.
