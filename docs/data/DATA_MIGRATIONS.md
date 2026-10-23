# SQLite schema migration policy

Implemented upgrades: v1 collections → v2 media/ingest → v3 workflows → v4 discovered artists. Prior migrations use normalized SHA-256 checksums, transactional upgrade and a SQLite online backup before upgrading existing catalogs. See [workflow delivery](../planning/WORKFLOW_DELIVERY.md).

Migration 004 rebuilds collections with paired nullable source identity/count, preserving local IDs, the autoincrement high-water mark and dependent media/job references. Foreign-key enforcement is temporarily disabled before the immediate migration transaction; foreign_key_check and quick_check must pass before commit, and enforcement is restored before returning a repository. Existing v1–v3 catalogs receive a consistent pre-version SQLite backup. Reverse media association/source indexes support batch loading and deletion. Artist backfill runs after file/deletion recovery, in one transaction over all surviving source records; raw source metadata, current edits and text artifacts are preserved. Earlier boundary notes below describe v1 history.

Every schema change is a numbered, immutable migration with a checksum. Never edit a migration already shipped; add a forward migration. Store schema version and migration history in the database. Enable foreign keys on every connection and configure a documented busy timeout. Transactions must be explicit for multi-row state changes.

## Upgrade protocol

1. Detect current schema version before opening normal repositories.
2. Create a consistent database backup/snapshot using SQLite-supported mechanisms; do not copy a live WAL database file alone.
3. Apply each migration transactionally where SQLite permits.
4. Run integrity checks and required data invariants.
5. Commit the new schema version only after successful migration.
6. If migration fails, preserve the original database and report a recovery path; never continue with a partially upgraded schema.

## Data integrity

Use foreign keys for ownership relations, explicit unique indexes for provider identity and exact duplicate keys, and check constraints for enumerated states where practical. Define whether nullable fields mean unknown, not applicable, or not yet fetched. Avoid storing derived counts as authoritative data. Use UTC timestamps in a documented format and never rely on local display strings for ordering.

## Migration tests

- Fresh database from an empty directory
- Upgrade from every supported previous schema fixture
- Interrupted migration and reopen
- Duplicate provider IDs and MD5 conflict handling
- Foreign-key integrity and orphan detection
- Unicode paths and malformed/legacy text metadata
- Backup/restore across schema versions

Migration fixtures must contain synthetic test data, not user media or credentials.

## Current adapter boundary

The first catalog migration creates `schema_migrations` and `collections` in an immediate transaction and sets `user_version=1`. Its SHA-256 uses UTF-8 SQL after CRLF-to-LF normalization, so Windows/Git line endings do not create incompatible checksums. Connections enable foreign keys and a 3-second busy timeout. Concurrent fresh opens serialize schema inspection and creation; reopen checks the immutable checksum and SQLite `quick_check` before serving records.

Version 1 is the only supported version. A nonempty unversioned database, higher version or checksum mismatch produces a schema error without rebuilding or erasing the database. This is fresh initialization/reopen support, not a complete upgrade runner. Backup-before-upgrade, shipped previous-version fixtures and interrupted forward-upgrade tests must be added when a v2 migration exists. Do not change migration 001 to add media tables; add a forward migration and update deletion ownership safeguards.
