# ModelScope backup and restore

## Manual command-line backup

Use the [backup script](../../scripts/modelscope_backup.py) from the machine that holds the BDM catalog. Close BDM desktop and stop any BDM CLI or MCP jobs before the backup. Run one backup at a time for each dataset branch.

On each Windows computer, open **系统设置 → 魔搭备份** and save the private dataset ID and Access Token. BDM writes the dataset ID to `config/modelscope.json` beside the project or portable install and stores the token in Windows Credential Manager. When running the script from a separate copy of the project, use `--config` with the portable install's configuration file.

```powershell
python -m pip install 'modelscope-hub>=0.7.1,<1' pywin32-ctypes
python scripts/modelscope_backup.py check
python scripts/modelscope_backup.py backup
python scripts/modelscope_backup.py restore .\restored-data
```

Use a private ModelScope **dataset** and a write-capable Access Token for backup. To bypass the desktop settings, use `--repo owner/name` or `MODELSCOPE_DATASET_ID`, and provide `MODELSCOPE_TOKEN` in the process environment or enter the token at the hidden prompt. The `check` command validates repository access and visibility. A successful backup and restore verify write access and readback.

`backup` defaults to the project's `data` folder. Use `backup --data 'D:\BDM\data'` for a portable install and repeat `--library 'D:\ImageLibrary'` for each additional or former managed-media root. Supply the same root paths that BDM used. `restore` defaults to the latest complete version and accepts `--id` for a specific version. Its destination must be a new folder under an existing parent. Restored SQLite media paths point to `library`, `library-1`, and subsequent restored roots. To try the restored data in the desktop app, restore directly to the `data` path of a separate portable BDM install. The restore command leaves the current catalog in place.

Each snapshot covers SQLite and its catalog-referenced working images, thumbnails, three text sidecars, current edit masks and known retained source originals. This first version includes current images and available source originals so a small library can be restored without re-downloading from the original providers. It excludes desktop `app.ron`, Windows Credential Manager, runtime logs, model weights and transient files. Set desktop preferences and logins again when needed.

The dataset stores shared files under `bdm/objects/`, immutable manifests with sizes and SHA-256 under `bdm/backups/`, and the current pointer at `bdm/LATEST.json`. Each backup verifies remote files by reading them back one at a time. Matching objects are reused after verification. An interrupted backup can be rerun to reuse verified files. Restore verifies files and the SQLite database in a temporary directory before moving it to the requested destination. Local-library data and past remote versions are retained. The CLI is manual. Desktop controls, weekly scheduling, remote retention and platform quota validation remain pending.

## Product requirements

Manual backup to a private ModelScope dataset repository. Include current valid original images, consistent SQLite snapshot, all three sidecars, artist/character/tag relations, review/quality data, DINO embeddings and identity relations, plus non-sensitive settings and schema/format versions. Never back up plaintext API keys or session credentials.

Normal originals are the only persistent local image copies. Do not build a completed local ZIP/TAR/shard archive or second full image library. Prefer streaming originals. If temporary shards are required, target 1 GB and delete only after remote confirmation and verification. Insufficient disk space means stop and alert. 1 GB is an application target, not a verified ModelScope limit.

Each version has an immutable manifest and SHA-256 checksums. Incremental uploads skip unchanged verified objects, but every successful version has a complete latest-state manifest and consistent DB snapshot. Update LATEST.json only after all required objects verify. Local deletions are excluded from the next manifest; historical remote objects may remain and are not auto-deleted.

State flow: CREATED → SNAPSHOTTING → PLANNING → UPLOADING → VERIFYING → COMPLETED. Failure/cancel preserves prior latest pointer. Restore into a temporary destination, restore DB and only manifest-referenced media, verify hashes/sizes/SQLite integrity/relations/text, report missing/corrupt objects, switch only after full validation, and preserve rollback data.

Remote layout: backups/<backup_id>/backup.json, database/app.sqlite3, images/, checksums.sha256, plus root LATEST.json. Before claiming support, test private-repo permissions, size limits, streaming/chunk/resume, commit semantics, remote readback/hash, quotas/rate limits, retries/timeouts and history behavior against the actual platform.
