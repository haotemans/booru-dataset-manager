# Captions and tags

Three independent UTF-8 sidecars per basename: image.raw_tags.txt (source provenance, preserve order/text, never AI-rewrite); image.tags.txt (meaning-preserving normalized tags); image.txt (factual natural-language training caption). Each artifact has its own version, hash, source/model/prompt config, state, error and review status. Unselected targets are untouched; a failed target does not roll back successful targets; reviewed content is never silently overwritten.

Preset 默认数据集优化: artist enabled, local formatter emits @artist name and never sends artist name to AI; character enabled, canonical library name in plain format and never AI-rewritten; tags enabled, AI processes ordinary tags only and protected names are merged locally; caption enabled, factual training description. Artist-only must not call AI. Caption-only must not change tags/artist/character; tags-only must not change caption.

Classify artist, character, work-specific, anatomical/technical, visual feature, ordinary descriptive, and ambiguous/conflicting terms. Strong protection forbids deletion/generalization; conditional mapping requires reviewed canonical mapping; ordinary normalization permits meaning-preserving changes only. Unknown terms stay unchanged and require review. Never infer domain-term definitions without trusted evidence or human confirmation. Use exact normalized matching and explicit aliases, not naive substring matching.

Validate protected-term coverage, approved mappings, unauthorized replacements, unsupported additions, raw-source immutability and association stability. AI semantic judgement is supplemental, not the sole approval criterion. Write via temp file and atomic replace where supported; preserve prior valid versions.
