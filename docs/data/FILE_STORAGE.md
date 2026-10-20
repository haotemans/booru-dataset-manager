# Managed file storage and crash recovery

## Goals

The managed library stores one canonical physical original per imported media record unless the user explicitly imports a distinct file. Artist/character views are associations, not separate copies. Thumbnails and derived previews are disposable caches and must never be treated as originals.

## Path rules

Use a stable internal media ID for managed paths; do not derive storage paths directly from untrusted tags, artist names, source filenames or AI output. Display/export filenames are generated separately and sanitized for the target filesystem. Reject path traversal, absolute paths, reserved Windows device names and invalid path components. Support Unicode and long paths where Windows permits. Preserve the original source filename as metadata, not as the canonical storage path.

## Import protocol

1. Create a unique temporary file in the managed library's filesystem/volume.
2. Stream the download into it with a configured byte limit and cancellation checks.
3. Flush and close the file; decode the actual image and validate dimensions and format.
4. Compute MD5 for exact duplicate lookup and SHA-256 for integrity.
5. Query the database uniqueness constraint. On duplicate, discard the temporary file and record source provenance against the existing media when appropriate.
6. Move the verified temporary file to a deterministic final path using an atomic same-volume rename where supported.
7. Commit the media row and source records in a transaction, with a durable operation journal that allows recovery if the process stops between steps.
8. Reconcile pending journal entries at startup. Never silently delete an unreferenced file until recovery has determined it is not a valid original.

## Failure recovery

Journal states should distinguish `TEMP_CREATED`, `DOWNLOADED`, `VALIDATED`, `HASHED`, `FILE_PROMOTED`, `DB_COMMITTED`, `CLEANUP_COMPLETE`, and terminal failure. Recovery must be idempotent. A file promoted without a database row is either reattached using the journal's verified metadata or moved to a recoverable quarantine. A database row whose file is missing is marked `storage_missing` and surfaced to the user; do not fabricate an empty replacement.

## Sidecar writes

Write each sidecar to a sibling temporary path, flush/close, then replace the prior file atomically where supported. Preserve the old version in version history before replacement. Commit metadata about the new version only after the new file is durable; if the database commit fails, recovery reconciles the file and version row. Never truncate the current file before the replacement is ready.

## Deletion

Logical deletion first marks media pending deletion and removes it from new export/backup manifests. Move the file to a quarantine/trash location before final database cleanup when possible. Keep enough journal data to recover after interrupted deletion. Shared source records, tags, artist/character links and visual identity relations must be handled according to explicit deletion policy; removing an association is not deleting the media.
