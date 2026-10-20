# Dashboard statistics contract

SQLite aggregations are the source of truth. Cards: total images, raw-tag coverage, optimized-tag coverage, caption coverage, all-three-text coverage, pending review, unsatisfactory count, and disk usage split into originals, sidecars and thumbnails. Coverage counts only current, valid, non-empty text; whitespace-only and invalid UTF-8 content does not count. `all three` is the intersection, not the sum of the individual counts. Missing categories overlap and must never be summed as unique missing images.

Coverage formula: valid current artifacts for a target / media in the selected scope × 100%. The UI must show numerator and denominator, not percentage alone. Clicking a metric opens the corresponding filtered list using the same scope and query semantics as the card.

Artist and character summaries include media count, raw tags, optimized tags, captions and all-three-complete count. Association-level totals can exceed global unique media count because media can be linked to multiple entities. Label the aggregation basis explicitly.

Cache only after profiling demonstrates a need. If cached, invalidate on media import/deletion, sidecar version change, association change, review change and scope-changing migration. Add tests for empty scope, overlapping missing states, duplicate associations and invalid/empty sidecars.
