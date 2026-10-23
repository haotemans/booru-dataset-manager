# Managed file storage and crash recovery

Current code stores static originals in content-addressed managed directories, preserves raw sidecars and SQLite source/collection associations, materializes tags/caption artifacts atomically, and uses import/deletion journals for recovery. Permanent media and collection-image cleanup is connected. The text below also contains earlier requirements; [workflow delivery](../planning/WORKFLOW_DELIVERY.md) records current behavior and outstanding hardening.

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

Confirmed inpainting replaces the current managed image in its existing source format and updates its current SHA-256, thumbnail and SQLite metadata. `StoredMedia.source_sha256` keeps the downloaded source hash when import normalization changes the stored bytes. With the download setting enabled, transparent PNG/WebP files are composited onto white before promotion; the source hash and source-post metadata remain tied to the downloaded bytes, while the current hash covers the promoted file. `StoredMedia.edit` keeps immutable original-source hash plus parent/output hashes, model revision/checksum, current mask path, coordinates and timestamp; source-post metadata and text artifacts are preserved. `inpainting-mask.png` is the current mask. `.repair-journal.json` is atomically written before promotion of staged image/thumbnail/mask files, and `recover_repairs` reconciles a confirmed replacement before the catalog loads. It does not create a retained archive of image versions. Uncommitted previews live in a disposable temporary directory; cancelling, closing or switching image disposes the preview. Source re-fetch and remote edit backup remain pending.

Confirmed crop uses the same journal and source-format replacement, with `operation=crop`, `model=local-crop-v1`, pixel rectangle and current dimensions. It stages image/thumbnail only; crop's mask path is empty and obsolete inpainting-mask.png is removed after commit. Source-post metadata stays immutable, and follow-up edits decode the current dimensions. Preview alone never changes managed files. Under the later unified editor decisions, crop retains one current local output and no pre-crop copy.

Write each sidecar to a sibling temporary path, flush/close, then replace the prior file atomically where supported. Preserve the old version in version history before replacement. Commit metadata about the new version only after the new file is durable; if the database commit fails, recovery reconciles the file and version row. Never truncate the current file before the replacement is ready.

## Deletion

D-080–D-083 require an explicit candidate/impact confirmation followed by immediate permanent deletion, with no recycle bin or undo copy. Exclude confirmed candidates from new export/backup manifests and use an idempotent operation journal for interrupted file/database cleanup and accurate partial-failure reporting. Delete each canonical original and its owned sidecars, thumbnails, review/analysis records and associations; preserve other images and shared tag definitions/models. Do not delete an entire visual identity group when only one image is selected.

Under D-134, the Artist/Character list's Delete command includes the selected records' associated canonical images, even when another collection references the same image. Freeze and show the union of image IDs once in the confirmation, remove each image and all its view references, and update remaining collections' covers/counts. Other collection records and their unrelated images survive. Merely editing an association is a separate action and must not be used to implement this Delete command. The desktop exercises media cascades only in-memory. The separate v1 collection catalog supports transactionally deleting zero-image records; all media filesystem/database steps above still require production adapters. See [collection workflow](../features/COLLECTION_CATALOG.md).
