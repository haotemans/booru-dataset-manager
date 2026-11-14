#!/usr/bin/env python3
"""Read-only artist dataset preflight. Does not download or upload images."""
import argparse
import json
import math
import os
import time
from datetime import datetime
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import Request, urlopen

RATIOS = (0.5, 9/16, 2/3, 3/4, 4/5, 1.0, 5/4, 4/3, 3/2, 16/9, 2.0)
STATIC = {"jpg", "jpeg", "png", "webp"}


def evaluate(post):
    w, h = int(post.get("image_width") or 0), int(post.get("image_height") or 0)
    item = {"id": post.get("id"), "size": [w, h], "artist": post.get("tag_string_artist") or ""}
    if post.get("rating") != "g" or post.get("is_deleted"):
        return "restricted_or_deleted", item
    if post.get("file_ext") not in STATIC:
        return "unsupported_format", item
    if min(w, h) < 576 or w * h < 550_000:
        return "small_resolution", item
    try:
        year = datetime.fromisoformat(str(post["created_at"]).replace("Z", "+00:00")).year
    except (KeyError, ValueError, TypeError):
        return "unknown_upload_date", item
    if year < 2020 or year > 2026:
        return "outside_upload_date_window", item
    ratio = w / h
    scale = min(1.0, 2048 / max(w, h))
    sw = max(32, int(w * scale) // 32 * 32)
    sh = max(32, int(h * scale) // 32 * 32)
    item.update({
        "danbooru_upload_year": year, "year_confidence": "upload_date_only",
        "planned_stored_size": [sw, sh],
        "candidate_ratio": min(RATIOS, key=lambda r: abs(math.log(ratio / r))),
        "crop_area_percent": round(100 * (1 - sw * sh / (w * scale * h * scale)), 3),
        "aesthetics": "not_verified", "rights": "not_verified",
    })
    return "candidate", item


def inspect_dataset(dataset_id, token):
    if not dataset_id or "/" not in dataset_id or not token:
        raise ValueError("Missing MODELSCOPE_DATASET_ID2 and/or MODELSCOPE_TOKEN")
    from modelscope_hub import HubApi
    api = HubApi(token=token)
    info = api.get_repo(dataset_id, "dataset")
    visibility = info.get("visibility") if isinstance(info, dict) else getattr(info, "visibility", None)
    if hasattr(visibility, "value"):
        visibility = visibility.value
    if str(visibility).lower() not in ("private", "1"):
        raise ValueError("Expected a private ModelScope dataset; refusing public media destination")
    files = list(api.list_repo_files(dataset_id, "dataset"))
    paths = [f.get("path", "") if isinstance(f, dict) else getattr(f, "path", str(f))
             for f in files]
    return {"private_verified": True, "files_count": len(paths),
            "files_preview": paths[:15]}


def run(artists, max_posts=20, pause=1.1):
    if len(set(artists)) != len(artists) or not artists:
        raise ValueError("Artist pilot list must be unique and non-empty")
    if not 1 <= max_posts <= 200:
        raise ValueError("max-posts must be 1..200")
    result = {
        "mode": "read_only", "artist_count": len(artists), "artists": {},
        "notes": [
            "created_at refers to Danbooru upload time, not the artwork's original release.",
            "No media, captions, BBoxes, aesthetics or legal rights have been verified.",
            "No upload or download of media is performed.",
        ],
    }
    for index, artist in enumerate(artists):
        if index:
            time.sleep(pause)
        q = urlencode({"tags": artist + " rating:g", "limit": max_posts})
        req = Request("https://danbooru.donmai.us/posts.json?" + q,
                      headers={"User-Agent": "BooruDatasetManager-ArtistPreflight/0.1"})
        with urlopen(req, timeout=30) as response:
            posts = json.load(response)
        if not isinstance(posts, list):
            raise ValueError("Unexpected Danbooru API response")
        stats, previews = {}, []
        for post in posts:
            label, item = evaluate(post)
            stats[label] = stats.get(label, 0) + 1
            if label == "candidate":
                previews.append(item)
        result["artists"][artist] = {"scanned": len(posts), "counts": stats, "preview": previews[:10]}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artists", type=Path, default=Path("datasets/artists/pilot_artists.json"))
    parser.add_argument("--max-posts", type=int, default=20)
    parser.add_argument("--output", type=Path, default=Path("data/artist-preflight/report.json"))
    parser.add_argument("--verify-modelscope", action="store_true")
    args = parser.parse_args()
    artists = json.loads(args.artists.read_text(encoding="utf-8"))["verified_artist_tags"]
    report = run(artists, args.max_posts)
    if args.verify_modelscope:
        report["modelscope"] = inspect_dataset(
            os.environ.get("MODELSCOPE_DATASET_ID", ""),
            os.environ.get("MODELSCOPE_TOKEN", ""))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print("Read-only artist preflight complete:", len(artists), "artist tags; report:", args.output)


if __name__ == "__main__":
    main()
