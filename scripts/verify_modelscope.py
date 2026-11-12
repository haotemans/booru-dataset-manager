#!/usr/bin/env python3
"""Verify private ModelScope dataset read/write access without exposing tokens."""
import hashlib
import os
import tempfile
from pathlib import Path
from modelscope_hub import HubApi

repo = os.environ.get("MODELSCOPE_DATASET_ID", "").strip()
token = os.environ.get("MODELSCOPE_TOKEN", "").strip()
if not repo or not token:
    raise SystemExit("MISSING_CONFIG: MODELSCOPE_DATASET_ID or MODELSCOPE_TOKEN is not configured")
if "/" not in repo or repo.endswith("/"):
    raise SystemExit("INVALID_DATASET_ID: expected owner/pixelart")

api = HubApi(token=token)
info = api.get_repo(repo, repo_type="dataset")
visibility = getattr(info, "visibility", None)
private = getattr(info, "private", None)
if not (private is True or visibility in (1, "1", "private")):
    raise SystemExit("DATASET_NOT_PRIVATE: verify visibility in ModelScope before uploading any training media")
print("DATASET_FOUND_PRIVATE", repo, flush=True)

payload = b"pixelart-modelscope-readwrite-check-v1\n"
digest = hashlib.sha256(payload).hexdigest()
remote = "pixel-art/diagnostics/connection-check-v1.txt"
api.upload_file(repo, "dataset", payload, remote, commit_message="Verify dataset uploader connection", revision="master", disable_tqdm=True)
with tempfile.TemporaryDirectory() as directory:
    result = api.download_file(repo, "dataset", remote, revision="master", local_dir=directory, force=True, expected_sha256=digest)
    if hashlib.sha256(Path(result).read_bytes()).hexdigest() != digest:
        raise SystemExit("REMOTE_VERIFY_FAILED")
print("UPLOAD_READBACK_VERIFIED", flush=True)
