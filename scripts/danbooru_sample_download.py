#!/usr/bin/env python3
"""One-off Danbooru pixel art original-media download and verification."""
import hashlib
import json
import re
import time
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import Request, urlopen

ROOT = Path("data/danbooru-sample")
TRAIN = ROOT / "train"
META = ROOT / "metadata"
TRAIN.mkdir(parents=True, exist_ok=True)
META.mkdir(parents=True, exist_ok=True)
HEADERS = {
    "User-Agent": "Booru-Dataset-Manager sample/0.1 (https://github.com/haotemans/booru-dataset-manager)",
    "Accept": "application/json"
}
SEARCHES = [
    ("image", "pixel_art rating:g", 12, {"png", "jpg", "jpeg", "webp"}),
    ("animation", "pixel_art rating:g filetype:gif", 4, {"gif"}),
]
SUBJECTS = {
    "1girl": "one girl", "2girls": "two girls", "1boy": "one boy",
    "solo": "a single character", "animal": "an animal",
    "landscape": "a landscape", "scenery": "a scene",
    "cat": "a cat", "dog": "a dog", "bird": "a bird",
    "flower": "flowers", "tree": "a tree", "house": "a house",
}
IGNORE = {"pixel_art", "highres", "absurdres", "lowres", "tagme",
          "animated", "animated_gif", "official_art", "revision", "sample"}

def fetch(url, maximum):
    with urlopen(Request(url, headers=HEADERS), timeout=30) as response:
        data = response.read(maximum + 1)
        if len(data) > maximum:
            raise ValueError("oversized file")
        if "text/html" in response.headers.get("Content-Type", ""):
            raise ValueError("HTML returned instead of expected resource")
        return data

def caption(post):
    tags = post.get("tag_string_general", "").split()
    subjects = list(dict.fromkeys(SUBJECTS[tag] for tag in tags if tag in SUBJECTS))[:2]
    detail_tags = [tag for tag in tags if tag not in IGNORE and tag not in SUBJECTS
                   and not tag.endswith("_name") and not re.match(r"^\d", tag)][:7]
    details = [tag.replace("_", " ") for tag in detail_tags]
    prefix = "A pixel-art animation" if post.get("file_ext") == "gif" else "A pixel-art illustration"
    sentence = prefix + (" depicting " + " and ".join(subjects) if subjects else "") + "."
    if details:
        sentence += " Visible details include " + ", ".join(details) + "."
    return sentence + "\n"

def main():
    found, errors, seen = [], [], set()
    for category, tags, needed, valid_extensions in SEARCHES:
        count = 0
        for page in (1, 2, 3):
            if count == needed:
                break
            query = urlencode({"tags": tags, "limit": 100, "page": page})
            try:
                posts = json.loads(fetch("https://danbooru.donmai.us/posts.json?" + query, 4_000_000))
                if not isinstance(posts, list):
                    raise ValueError("unexpected API response")
            except (HTTPError, URLError, TimeoutError, ValueError) as exc:
                errors.append({"stage": "api", "tags": tags, "error": str(exc)})
                print("Search error:", exc, flush=True)
                break
            print("Found", len(posts), "candidates for", category, flush=True)
            time.sleep(1.1)
            for post in posts:
                if count == needed:
                    break
                pid, ext = post.get("id"), str(post.get("file_ext", "")).lower()
                if (not pid or pid in seen or post.get("rating") != "g"
                    or ext not in valid_extensions or post.get("is_deleted")
                    or (post.get("file_size") or 0) > 12_000_000):
                    continue
                md5 = str(post.get("md5") or "")
                if not re.fullmatch("[0-9a-f]{32}", md5):
                    continue
                url = post.get("file_url") or (
                    "https://cdn.donmai.us/original/" + md5[:2] + "/" +
                    md5[2:4] + "/" + md5 + "." + ext)
                try:
                    data = fetch(url, 12_000_000)
                    if hashlib.md5(data).hexdigest() != md5:
                        raise ValueError("MD5 mismatch")
                    if ext == "gif" and data[:6] not in (b"GIF87a", b"GIF89a"):
                        raise ValueError("GIF format mismatch")
                    if ext == "png" and not data.startswith(b"\x89PNG\r\n\x1a\n"):
                        raise ValueError("PNG format mismatch")
                    if ext in ("jpg", "jpeg") and not data.startswith(b"\xff\xd8\xff"):
                        raise ValueError("JPEG format mismatch")
                    if ext == "webp" and not (data.startswith(b"RIFF") and data[8:12] == b"WEBP"):
                        raise ValueError("WebP format mismatch")
                    (TRAIN / (str(pid) + "." + ext)).write_bytes(data)
                    (TRAIN / (str(pid) + ".txt")).write_text(caption(post), encoding="utf-8")
                    (META / (str(pid) + ".json")).write_text(
                        json.dumps(post, ensure_ascii=False, indent=2), encoding="utf-8")
                    found.append({
                        "id": pid, "category": category, "rating": "g",
                        "media": "train/" + str(pid) + "." + ext,
                        "caption": "train/" + str(pid) + ".txt",
                        "metadata": "metadata/" + str(pid) + ".json",
                        "md5": md5, "size_bytes": len(data),
                        "post_url": "https://danbooru.donmai.us/posts/" + str(pid),
                        "source_url": post.get("source") or "",
                        "rights_verified": False, "caption_verified": False
                    })
                    seen.add(pid)
                    count += 1
                    print("Downloaded", pid, ext, len(data), "bytes", flush=True)
                except (HTTPError, URLError, TimeoutError, ValueError) as exc:
                    errors.append({"stage": "download", "id": pid, "error": str(exc)})
                    print("Skip", pid, str(exc), flush=True)
                time.sleep(0.9)
        print(category, count, "/", needed, flush=True)
    (ROOT / "manifest.jsonl").write_text(
        "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in found),
        encoding="utf-8")
    (ROOT / "errors.json").write_text(
        json.dumps(errors, ensure_ascii=False, indent=2), encoding="utf-8")
    (ROOT / "README.md").write_text(
        "# Danbooru pixel-art download verification\n\n"
        + "Media downloaded and MD5 verified: " + str(len(found)) + "\n\n"
        + "train/ contains original media and one matching .txt file per item.\n"
        + "metadata/ contains original Danbooru JSON including unmodified Tags.\n"
        + "manifest.jsonl lists checksums, source links, and review flags.\n"
        + "GIF is original with native timing. No 512px or 8fps preprocessing.\n"
        + "Captions are heuristic natural-language drafts; NOT visually verified.\n"
        + "Rights and training permissions are NOT verified; review before use.\n",
        encoding="utf-8")
    print("DONE", len(found), "media files,", len(errors), "errors", flush=True)
    if len(found) == 0:
        raise SystemExit("No media downloaded; see errors.json")

if __name__ == "__main__":
    main()
