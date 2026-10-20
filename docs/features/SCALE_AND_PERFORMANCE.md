# Scale and performance design: 100k–1M media

## Core principles

The target is 100,000–1,000,000 media records on a Windows desktop. This is a design target, not a verified capacity claim. Performance acceptance must be measured on representative hardware and data.

- Keep SQLite as the metadata source of truth; use migrations, foreign keys, query plans and indexes for actual filter paths.
- Paginate galleries and candidate lists. Never materialize the entire media table or embedding matrix in the UI process.
- Run downloads, image decoding, embedding generation, similarity retrieval, exports and backups as bounded background jobs. UI updates must be incremental and cancellation-aware.
- Bound network concurrency, decoder memory, model batch size, queue length and temporary disk usage. Backpressure should pause discovery/download rather than grow unbounded queues.
- Separate durable records from rebuildable caches such as thumbnails and ANN indexes.

## Index strategy

Start with indexed SQLite queries and benchmark expected filters. Exact MD5 lookup uses a unique/indexed key with explicit collision handling. Add an approximate-nearest-neighbor index only after testing candidate recall, index build/rebuild cost, memory, incremental updates and Windows packaging. Store model/preprocessing version alongside every embedding; incompatible embeddings must not be mixed in one index.

## Query and UI behavior

- Use keyset pagination for large stable lists where feasible; define deterministic sort keys.
- Fetch only visible rows plus a bounded prefetch window.
- Cache thumbnails by media ID plus source content/version; invalidate on replacement.
- Use query plans and representative synthetic datasets to test artist/character/tag/review/export filters.
- Show indeterminate progress when total work is unknown. Do not fabricate percentages.

## Benchmark suite

Record hardware/OS, dataset size, embedding dimensions, model/runtime versions and cache state. Measure cold/warm gallery latency, filtered query latency, exact-hash lookup, candidate retrieval latency, embedding throughput, peak resident memory, index size, startup time, migration duration, export throughput and backup verification throughput. Report p50/p95 and dataset shape; avoid a single unqualified speed number.

## Initial acceptance targets

Treat these as proposed targets for validation, not current performance claims: common indexed filters should feel interactive; scrolling should not load full-resolution originals for every visible card; cancellation should stop scheduling new work promptly; and long-running tasks should survive application restart. Numeric latency budgets must be set after baseline benchmarks on the minimum supported Windows hardware.
