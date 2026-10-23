# Database schema: implemented catalog and planned media core

Local inpainting stores the current `EditRecord` inside each media row's JSON metadata without changing schema version 3. It records operation/model, source/parent/current output SHA-256, current mask asset, source-pixel regions and timestamp. Confirmed repair updates current media hash and invalidates visual features/buckets/duplicate candidates/quality before future analysis. Source IDs/MD5/post metadata, collection associations and text artifacts are unchanged; import reuse distinguishes original-source SHA-256 from the edited current hash. The file promotion journal is a sibling `.repair-journal.json`, recovered before loading media; audit rows record intent and commit.

SQLite v4 production tables include media, media_sources, collection_media, import/deletion journals, ingest/workflow jobs, text artifact versions, tag rules, analysis state, visual features/buckets/candidates, quality metadata and audit events. See [delivery status](../planning/WORKFLOW_DELIVERY.md).

Migration [`004_discovered_artists.sql`](../../crates/infrastructure/src/migrations/004_discovered_artists.sql) makes `collections.source_tag_id` and `remote_post_count` nullable together: NULL means an artist discovered from a source post without a separate remote tag lookup. Positive IDs/nonnegative totals and existing uniqueness constraints remain enforced when present. Local ID/name/category remain authoritative. `CollectionRecord.source` and the desktop API count preserve unknown as an Option. The migration adds `collection_media(media_id,collection_id)` and `media_sources(media_id)` indexes and retains dependent rows and the collection sequence. Historical v1 details below apply to the original migration, not the current version.

SQLite metadata/index is the source of truth. Version all schema changes as migrations, enable foreign keys, use transactions, and test fresh and upgraded databases. Do not copy a live WAL database as if it were a consistent snapshot.

Planned families: media; source_records; artists and characters; tags and many-to-many associations; text_artifacts/captions (raw_tags, optimized_tags, caption, version/hash/path/source/model/prompt/state/review/current flag); protected_terms; tag_normalization_rules; metadata_optimization_runs; metadata_validation_results; media_reviews; media_quality_flags; visual_embeddings; visual_identity_groups/members; duplicate_relations; export_profiles/jobs/job_items; backup_profiles/jobs/job_items/snapshots.

Exact duplicate correctness must use database uniqueness over normalized MD5/content identity. In-memory sets are caches only. Define collision handling and migration behavior for existing duplicates.

## Implemented collection catalog v1

The authoritative v1 collection schema is [`001_collections.sql`](../../crates/infrastructure/src/migrations/001_collections.sql); v2/v3 add media and workflow tables. The adapter is [`catalog.rs`](../../crates/infrastructure/src/catalog.rs). The default database is project-local `data/catalog.sqlite3`, separately from `data/app.ron` preferences and `data/library/` media. Preview/capture do not open it.

| Table / column | Meaning and invariant |
|---|---|
| `schema_migrations.version/checksum/applied_at` | Numbered migration history, SHA-256 over UTF-8 SQL with CRLF normalized to LF, UTC application timestamp |
| `collections.id` | Positive autoincrement local identity; reverse ID order displays newest added first |
| `provider/source_tag_id` | Currently `danbooru` and a positive remote Tag ID; composite uniqueness |
| `category/tag` | Category 1 (artist) or 4 (character), nonempty canonical Tag; `(provider, tag)` unique |
| `remote_post_count/fetched_at` | Nonnegative source-count snapshot and initial-add UTC timestamp; not an authoritative local-media count |

`PRAGMA user_version` is 1. Initialization and inserts use immediate transactions with a 3-second busy timeout; foreign keys are enabled on each connection. Existing source identities return the original record unchanged. Confirmed deletion of zero-image records is all-or-nothing and rejects missing/wrong-category IDs. The v1 schema contains no media or association table; future migrations must wire coordinated deletion before permitting media-owning records to be removed.

The adapter runs `quick_check` and validates the stored migration checksum. Unknown versions, altered checksums and nonempty unversioned databases are rejected and preserved. Production media schema, MD5 uniqueness, SQL-backed pagination, WAL snapshot/restore and forward upgrades are still planned, not demonstrated by this catalog.

Download resolution configuration (2026-10-21): ingest-job and stored-media JSON include `minimum_short_side`. Jobs snapshot the saved desktop threshold at creation; import journals carry the media threshold for promoted-file recovery. Missing fields in existing JSON default to 800 px. No SQL table/checksum migration is required for this additive JSON metadata field.

Image editing (2026-10-22): optional `StoredMedia.edit` retains operation, model, original-source/parent/output hashes, current mask/regions and timestamp. Additive `edit.dimensions` stores current output width/height; older records fall back to `post.image_width/image_height`. `edit.crop` stores x/y/width/height for crop; crop has an empty mask path and no regions. Source-post dimensions remain unchanged. Confirmed edits use the recoverable sibling journal and invalidate visual caches. Export manifests include `current_dimensions` and `edit` separately from `source`. No SQL checksum/schema change is required.
