"""Relocate a stopped local catalog; verify copied bytes before publishing it.

The original directory is preserved until the caller explicitly cleans it up.
No credentials are read and no network requests are made.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    source = args.source.resolve(strict=True)
    destination = args.destination.resolve()
    if destination.exists() or source == destination or source in destination.parents:
        raise ValueError("destination must be a new separate directory")
    staging = destination.with_name(destination.name + ".relocation-staging")
    if staging.exists():
        raise ValueError("staging exists; inspect the previous migration first")
    database = source / "catalog.sqlite3"
    connection = sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)
    for table in ("import_journal", "deletion_journal"):
        if connection.execute(f"SELECT count(*) FROM {table}").fetchone()[0]:
            raise ValueError("recover outstanding file journals before migration")
    shutil.copytree(source, staging, ignore=shutil.ignore_patterns(
        "catalog.sqlite3", "catalog.sqlite3-wal", "catalog.sqlite3-shm"))
    copied = sqlite3.connect(staging / "catalog.sqlite3")
    connection.backup(copied)
    counts = {table: connection.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
              for table in ("media", "media_sources", "collections", "collection_media",
                            "ingest_jobs", "workflow_jobs", "text_artifacts")}
    connection.close()
    backup_dir = staging / "backups"
    backup_dir.mkdir(exist_ok=True)
    backup = sqlite3.connect(backup_dir / "pre-relocation.sqlite3")
    copied.backup(backup)
    backup.close()
    old_library = source / "library"
    new_library = destination / "library"

    def replace(value):
        if isinstance(value, dict):
            return {key: replace(item) for key, item in value.items()}
        if isinstance(value, list):
            return [replace(item) for item in value]
        if isinstance(value, str):
            path = Path(value)
            if path.is_absolute() and path.is_relative_to(old_library):
                return str(new_library / path.relative_to(old_library))
        return value

    with copied:
        for table in ("media", "import_journal", "deletion_journal", "ingest_jobs", "workflow_jobs"):
            for rowid, metadata in copied.execute(f"SELECT rowid,metadata FROM {table}").fetchall():
                value = replace(json.loads(metadata))
                if table in ("ingest_jobs", "workflow_jobs") and value.get("state") in ("running", "queued"):
                    value["state"] = "interrupted"
                copied.execute(f"UPDATE {table} SET metadata=? WHERE rowid=?",
                               (json.dumps(value, ensure_ascii=False), rowid))
        copied.execute("INSERT INTO audit_log(action,metadata) VALUES('relocate_data',?)",
                       (json.dumps({"source": str(source), "destination": str(destination)}),))
    preferences = staging / "app.ron"
    if preferences.exists():
        text = preferences.read_text(encoding="utf-8")
        # eframe serializes the app preferences as a RON string inside its storage map.
        for old, new in ((source, destination), (old_library, new_library)):
            for old_encoded, new_encoded in ((str(old), str(new)),
                                            (str(old).replace("\\", "\\\\"), str(new).replace("\\", "\\\\")),
                                            (str(old).replace("\\", "\\\\\\\\"), str(new).replace("\\", "\\\\\\\\"))):
                text = text.replace(old_encoded, new_encoded)
        preferences.write_text(text, encoding="utf-8")
    files = 0
    for original in old_library.rglob("*"):
        if original.is_file():
            target = staging / original.relative_to(source)
            if sha256(original) != sha256(target):
                raise ValueError("copied file checksum mismatch")
            files += 1
    for (metadata,) in copied.execute("SELECT metadata FROM media"):
        media = json.loads(metadata)
        original = staging / Path(media["path"]).relative_to(destination)
        thumbnail = staging / Path(media["thumbnail"]).relative_to(destination)
        if sha256(original) != media["sha256"] or not thumbnail.is_file():
            raise ValueError("catalog file integrity mismatch")
    for table, count in counts.items():
        if copied.execute(f"SELECT count(*) FROM {table}").fetchone()[0] != count:
            raise ValueError("catalog row count changed")
    if copied.execute("PRAGMA integrity_check").fetchone()[0] != "ok" or copied.execute("PRAGMA foreign_key_check").fetchall():
        raise ValueError("catalog integrity check failed")
    copied.close()
    (staging / "relocation.json").write_text(json.dumps({
        "source": str(source), "destination": str(destination), "counts": counts,
        "verified_library_files": files, "state": "verified"
    }, indent=2), encoding="utf-8")
    staging.rename(destination)
    print(json.dumps({"destination": str(destination), "media": counts["media"],
                      "verified_files": files, "state": "verified"}))


if __name__ == "__main__":
    main()
