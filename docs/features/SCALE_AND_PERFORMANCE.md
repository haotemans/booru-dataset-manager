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

Tag page optimization (2026-10-21): the desktop caches per-tag unique-image counts and normalization warnings until media/collection/tag-rule data changes. Idle repaints and scrolls reuse the same summary; filtered row indices are recomputed only for changed search/category/summary. Tag tables render one page of 20/50/100 entries and virtualize visible rows within that page; conflicts have their own pagination. Searches cover the full cached list before slicing. Filtering resets the tag page and scroll, shrinking lists clamps invalid pages, and stable tag IDs preserve checkbox identity. This is a desktop rendering optimization, not a database keyset-pagination implementation or a 50,000-image performance guarantee.

Download/dedup pipeline (2026-10-21): one ingestion worker and one visual worker can overlap network and CPU work. Visual dispatch fetches at most 64 pending IDs, oldest first, from SQLite and drains subsequent batches on completion; it does not scan the queue every UI frame. A process-wide per-catalog mutex coordinates short media mutation phases, while network, decoding, thumbnail preparation and DINO inference remain outside it. The durable queue can grow with the library; disk backpressure, model-process reuse across batches, incremental UI catalog queries, cross-process coordination and resource/throughput benchmarks remain future work.

Gallery optimization (2026-10-23): only visible artwork schedules thumbnail decoding on one worker, with bounded 8-item request/result channels, 16 MiB decode allocation limits and at most four texture uploads per frame. The 128-texture cache evicts the least recently used entry rather than clearing all entries. Failed decodes wait for invalidation, and generation tickets discard late replies after editing/deletion. Originals remain opt-in under D-140. Collection counts/covers are cached when associations change and updated after deletion. `list_media` loads all associations in one query rather than one prepare/query per image; tag association uses indexed name/category lookups, and legacy association repair scans all source provenance in one transaction. Ingest progress updates immediately but full-library refreshes are coalesced to at most once per 350 ms, with a final completion refresh. Full-library loading, SQLite integrity checks on opens and SQL paging remain larger-library limitations; these changes are not a 100k–1M capacity claim.
