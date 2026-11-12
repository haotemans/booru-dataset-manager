#!/usr/bin/env python3
"""Publish a verified pixel-art archive to a PRIVATE ModelScope dataset.

The repository and token must be configured outside source control. This script
does not grant permission to train on or redistribute submitted material.
"""
from __future__ import annotations

import argparse
import hashlib
import os
import tempfile
from pathlib import Path


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(4 * 1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def publish(archive: Path, state: Path, repo: str, token: str, revision: str) -> dict:
    if not archive.is_file() or not state.is_file():
        raise ValueError("Missing archive or progress index")
    if not token or "/" not in repo:
        raise ValueError("MODELSCOPE_TOKEN and MODELSCOPE_DATASET_ID are required")
    expected = digest(archive)
    checksum_file = archive.with_suffix(".tar.sha256")
    if not checksum_file.is_file() or checksum_file.read_text().split()[0] != expected:
        raise ValueError("Archive does not match local SHA256 manifest")

    from modelscope_hub import HubApi
    api = HubApi(token=token)
    # Reuse the verified ModelScope API to resolve the dataset and enforce privacy.
    info = api.get_repo(repo, repo_type="dataset")
    visibility = getattr(info, "visibility", None)
    private = getattr(info, "private", None)
    if not (private is True or visibility in (1, "1", "private")):
        raise ValueError("Refusing to publish third-party media to a non-private dataset")
    destination = "pixel-art/shards/" + archive.name
    uploaded = api.upload_file(
        repo, "dataset", str(archive), destination,
        commit_message="Add verified pixel-art shard " + archive.stem,
        revision=revision, disable_tqdm=True)
    if not uploaded:
        raise RuntimeError("ModelScope did not confirm the archive commit")

    # Verify that the remote content is present and byte-for-byte identical.
    with tempfile.TemporaryDirectory(prefix="pixel-art-verify-") as tmp:
        remote = Path(api.download_file(
            repo, "dataset", destination, revision=revision,
            local_dir=tmp, force=True, expected_sha256=expected))
        if digest(remote) != expected:
            raise RuntimeError("Remote archive read-back SHA256 mismatch")

    # Save the index only AFTER the complete archive was remotely verified.
    progress_commit = api.upload_file(
        repo, "dataset", str(state), "pixel-art/progress.json",
        commit_message="Checkpoint curated pixel-art data after " + archive.stem,
        revision=revision, disable_tqdm=True)
    if not progress_commit:
        raise RuntimeError("Could not checkpoint remote progress")
    return {"dataset": repo, "destination": destination, "sha256": expected}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--revision", default=os.getenv("MODELSCOPE_REVISION", "master"))
    args = parser.parse_args()
    result = publish(args.archive, args.state,
                     os.getenv("MODELSCOPE_DATASET_ID", ""),
                     os.getenv("MODELSCOPE_TOKEN", ""), args.revision)
    print("Verified persistent upload", result["destination"], result["sha256"])


if __name__ == "__main__":
    main()
