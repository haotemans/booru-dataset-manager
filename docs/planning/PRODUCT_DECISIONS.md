# Product decisions

This file records user-confirmed product decisions. A decision is not a claim that its feature is implemented.

## D-001 — Product scope

**Decision:** Build a complete integrated desktop tool covering source ingestion, media and identity management, metadata optimization, review, dataset export, dashboard statistics, and backup/restore.

**Implications:** Deliver in staged milestones, but keep a shared media library and shared application services. Avoid separate incompatible workflows for artist pages, character pages and global export. Preserve modular boundaries so source adapters, AI providers, model runtimes and backup transports can be added independently.

**Status:** Confirmed by user; architecture/documentation only.

## D-002 — Visual similarity policy

**Decision:** DINOv3 may automatically group visual-similarity candidates using a validated, versioned policy, but it must not delete original files. Users must be able to inspect and manually choose preferred images. The product should also support automatic selection of a recommended representative image where the configured policy and evaluation evidence justify it.

**Implications:** Distinguish visual grouping, representative selection and physical deletion as separate operations. Group membership does not imply byte-identical duplicates. A representative is a reversible dataset-selection decision, not a destructive cleanup. Exact byte duplicates may be deduplicated logically using database-enforced hashes while retaining source provenance.

**Status:** Confirmed by user; policy design only. Model thresholds and quality-ranking behavior are not yet calibrated or implemented.

## D-003 — Target library scale

**Decision:** Design for 100,000 to 1,000,000 media items.

**Implications:** Use SQLite indexes and batched queries; avoid loading the full library or all embeddings into UI memory. Separate embedding generation from interactive queries. Use paginated candidate retrieval, bounded queues, resumable jobs, cancellation, and measurable latency/memory budgets. Benchmark before selecting an approximate-nearest-neighbor index or adding a separate search service.

**Status:** Confirmed by user; performance has not been benchmarked.

## D-004 — Representative-image ranking and original retention

**Decision:** Rank representative candidates primarily by visual appeal and degree of completion, then by minimal visual distractions; images with no visible text are preferred. Keep all original files. Mark the recommended representative separately and let export settings choose representative-only or all variants.

**Implications:**
- Treat aesthetic/completion quality as the primary human preference, not file size, source priority, or embedding similarity.
- Prefer clean compositions with minimal distracting overlays or other visual clutter; visible text is a negative signal, with text-free images preferred when other quality factors are comparable.
- Use measurable technical signals (decode validity, blur/sharpness, compression artifacts, resolution and upscaling indicators) as supporting evidence, not as a replacement for aesthetic judgment.
- Show the recommendation and its reasons, allow manual override, and preserve decision history. Do not delete, overwrite, or silently quarantine non-representative originals.
- This is a ranking policy to evaluate, not a claim that aesthetic quality can already be measured reliably by an implemented model. Ambiguous or close calls should be surfaced for human review.

**Status:** User-confirmed preference; ranking implementation and evaluation remain pending.

## Deferred decisions

- Exact ranking weights, how aesthetic completion and visual clutter are measured, and whether some signals require a vision model or human labels.
- Whether automatic representative selection is enabled by default for newly discovered groups.
- Whether a visual group can contain different crops/poses/frames versus only near-duplicate versions. The relation types must remain distinct even if groups share one UI.
- Which DINOv3 artifact/runtime and nearest-neighbor index meet license, compatibility, memory and latency requirements.
