# SQLite schema migration policy

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
