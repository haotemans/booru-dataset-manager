# ModelScope backup and restore

Manual backup to a private ModelScope dataset repository. Include current valid original images, consistent SQLite snapshot, all three sidecars, artist/character/tag relations, review/quality data, DINO embeddings and identity relations, plus non-sensitive settings and schema/format versions. Never back up plaintext API keys or session credentials.

Normal originals are the only persistent local image copies. Do not build a completed local ZIP/TAR/shard archive or second full image library. Prefer streaming originals. If temporary shards are required, target 1 GB and delete only after remote confirmation and verification. Insufficient disk space means stop and alert. 1 GB is an application target, not a verified ModelScope limit.

Each version has an immutable manifest and SHA-256 checksums. Incremental uploads skip unchanged verified objects, but every successful version has a complete latest-state manifest and consistent DB snapshot. Update LATEST.json only after all required objects verify. Local deletions are excluded from the next manifest; historical remote objects may remain and are not auto-deleted.

State flow: CREATED → SNAPSHOTTING → PLANNING → UPLOADING → VERIFYING → COMPLETED. Failure/cancel preserves prior latest pointer. Restore into a temporary destination, restore DB and only manifest-referenced media, verify hashes/sizes/SQLite integrity/relations/text, report missing/corrupt objects, switch only after full validation, and preserve rollback data.

Remote layout: backups/<backup_id>/backup.json, database/app.sqlite3, images/, checksums.sha256, plus root LATEST.json. Before claiming support, test private-repo permissions, size limits, streaming/chunk/resume, commit semantics, remote readback/hash, quotas/rate limits, retries/timeouts and history behavior against the actual platform.
