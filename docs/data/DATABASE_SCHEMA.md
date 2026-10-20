# Database schema plan

SQLite metadata/index is the source of truth. Version all schema changes as migrations, enable foreign keys, use transactions, and test fresh and upgraded databases. Do not copy a live WAL database as if it were a consistent snapshot.

Planned families: media; source_records; artists and characters; tags and many-to-many associations; text_artifacts/captions (raw_tags, optimized_tags, caption, version/hash/path/source/model/prompt/state/review/current flag); protected_terms; tag_normalization_rules; metadata_optimization_runs; metadata_validation_results; media_reviews; media_quality_flags; visual_embeddings; visual_identity_groups/members; duplicate_relations; export_profiles/jobs/job_items; backup_profiles/jobs/job_items/snapshots.

Exact duplicate correctness must use database uniqueness over normalized MD5/content identity. In-memory sets are caches only. Define collision handling and migration behavior for existing duplicates.
