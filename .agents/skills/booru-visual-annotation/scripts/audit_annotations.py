"""Read-only checks of existing booru annotations; never a visual approval."""
import argparse
from contextlib import closing
import hashlib
import json
import math
from pathlib import Path
import re
import sqlite3

NON_VISUAL = {
    "absurdres", "highres", "lowres", "incredibly absurdres", "commentary",
    "commentary request", "chinese commentary", "japanese commentary",
    "english commentary", "copyright request", "artist request",
    "character request", "source request", "translation request", "tagme",
    "masterpiece", "best quality", "high quality",
}
TEXT_CLUES = {
    "text", "signature", "watermark", "weibo_watermark", "artist_name",
    "english_text", "chinese_text", "japanese_text", "speech_bubble",
    "newspaper", "book", "sign", "logo", "clothing_print", "sticker",
}
REGION_KINDS = {"dialog", "sign", "clothing_print", "watermark", "overlay", "sticker", "other"}


def identity_prefix(post, rules):
    aliases = rules.get("aliases", {})

    def canonical(name):
        original, seen = name, set()
        while name in aliases:
            if name in seen:
                return original
            seen.add(name)
            name = aliases[name]
        return name

    artists = {canonical(t) for t in post.get("tag_string_artist", "").split()}
    characters = {canonical(t) for t in post.get("tag_string_character", "").split()}
    for tag in post.get("tag_string", "").split():
        name = canonical(tag)
        category = rules.get("categories", {}).get(name)
        if category == 1:
            artists.add(name)
        elif category == 4:
            characters.add(name)
    present = characters.copy()
    edges = {}
    valid_hierarchy = True
    for relation in rules.get("characters", []):
        if not relation.get("verified"):
            continue
        base, variant = canonical(relation.get("base", "")), canonical(relation.get("variant", ""))
        if not base or not variant or base == variant or variant in edges.get(base, set()):
            valid_hierarchy = False
        edges.setdefault(base, set()).add(variant)
    for start in edges:
        pending, seen = list(edges[start]), set()
        while pending:
            name = pending.pop()
            if name == start:
                valid_hierarchy = False
                break
            if name not in seen:
                seen.add(name)
                pending.extend(edges.get(name, set()))
    for relation in rules.get("characters", []):
        if valid_hierarchy and relation.get("verified") and canonical(relation.get("variant", "")) in present:
            characters.discard(canonical(relation.get("base", "")))
    return ({"@" + t.replace("_", " ") for t in artists},
            {t.replace("_", " ") for t in characters})


def valid_region(region):
    if not isinstance(region, dict):
        return False
    text = region.get("text")
    box = region.get("bbox_1000_xyxy")
    if not isinstance(text, str) or not text.strip() or len(text.encode("utf-8")) > 512:
        return False
    if any(ord(c) < 32 for c in text) or region.get("kind") not in REGION_KINDS:
        return False
    legibility = region.get("legibility")
    if legibility not in {"readable", "partial", "unreadable", "decorative"}:
        return False
    if (legibility in {"unreadable", "decorative"}) != (text.strip() == "unreadable text"):
        return False
    if not valid_confidence(region.get("confidence")):
        return False
    for key, limit in [("language", 64), ("appearance", 1024)]:
        value = region.get(key)
        if value is not None and (not isinstance(value, str) or len(value.encode()) > limit or any(ord(c) < 32 for c in value)):
            return False
    return (isinstance(box, list) and len(box) == 4
            and all(type(v) is int for v in box)
            and 0 <= box[0] < box[2] <= 1000 and 0 <= box[1] < box[3] <= 1000)


def valid_confidence(value):
    return value is None or (type(value) in {int, float} and math.isfinite(value) and 0 <= value <= 1)


def valid_visual_region(region):
    if not isinstance(region, dict) or region.get("kind") not in {
        "subject", "background", "sticker", "overlay", "embedded_image", "detail_inset", "x_ray", "cross_section"
    }:
        return False
    proxy = {"text": "unreadable text", "kind": "other", "legibility": "unreadable",
             "bbox_1000_xyxy": region.get("bbox_1000_xyxy"), "confidence": region.get("confidence")}
    if not valid_region(proxy):
        return False
    return all(isinstance(region.get(k), str) and region[k].strip() and len(region[k].encode()) <= limit
               and not any(ord(c) < 32 for c in region[k]) for k, limit in [("description", 2048), ("relation", 512)])


def audit(catalog, ids, include_details=False):
    catalog = Path(catalog).resolve()
    if not catalog.is_file():
        raise FileNotFoundError(f"Catalog does not exist: {catalog}")
    ids = list(dict.fromkeys(ids))
    if not ids or len(ids) > 128 or any(type(i) is not int or i <= 0 for i in ids):
        raise ValueError("Provide 1–128 positive media IDs in the authorized scope")
    connection = sqlite3.connect(catalog.as_uri() + "?mode=ro", uri=True)
    try:
        connection.execute("PRAGMA query_only=ON")
        connection.execute("BEGIN")
        row = connection.execute("SELECT metadata FROM tag_rules WHERE id=1").fetchone()
        rules = json.loads(row[0]) if row else {}
        images = []
        for media_id in ids:
            item = {"id": media_id, "blocking": [], "review": [], "visual_semantics": "not_checked_by_script"}
            row = connection.execute("SELECT metadata FROM media WHERE id=?", (media_id,)).fetchone()
            if not row:
                item["blocking"].append("media_not_found")
                images.append(item)
                continue
            media = json.loads(row[0])
            artists, characters = identity_prefix(media["post"], rules)
            item["current_sha256"] = media["sha256"]
            artifacts = connection.execute(
                "SELECT kind,content,sha256,state FROM text_artifacts WHERE media_id=? AND is_current=1",
                (media_id,)).fetchall()
            by_kind = {}
            for kind, content, expected_sha, state in artifacts:
                if kind in by_kind:
                    item["blocking"].append("multiple_current_" + kind)
                by_kind[kind] = content
                raw = content.encode("utf-8")
                if hashlib.sha256(raw).hexdigest() != expected_sha:
                    item["blocking"].append(kind + "_artifact_hash_mismatch")
                suffix = ".tags.txt" if kind == "tags" else ".txt"
                sidecar = Path(media["path"]).with_suffix(suffix)
                if not sidecar.is_file() or sidecar.read_bytes() != raw:
                    item["blocking"].append(kind + "_sidecar_differs_from_database")
                if state != "complete":
                    item["review"].append(kind + "_not_complete")
                parts = [t.strip() for t in content.split(",")]
                if set(parts[:len(artists)]) != artists:
                    item["blocking"].append(kind + "_artist_prefix_or_order")
                start, end = len(artists), len(artists) + len(characters)
                if set(parts[start:end]) != characters:
                    item["blocking"].append(kind + "_character_prefix_or_order")
                body = ", ".join(parts[end:]).strip()
                if kind == "tags":
                    polluted = sorted(set(t.lower() for t in parts) & NON_VISUAL)
                    if polluted:
                        item["blocking"].append("non_visual_tags: " + ", ".join(polluted))
                    if any("_" in t for t in parts):
                        item["blocking"].append("underscores_in_training_tags")
                elif re.match(r"^(?:\d+(?:girls?|boys?)|solo|no humans|multiple (?:girls|boys))\b", body, re.I):
                    item["blocking"].append("caption_body_starts_as_tag_list")
            for kind in ["caption", "tags"]:
                if not by_kind.get(kind, "").strip():
                    item["blocking"].append("missing_or_empty_" + kind)
            row = connection.execute("SELECT provenance FROM media_quality WHERE media_id=?", (media_id,)).fetchone()
            try:
                details = json.loads(row[0]) if row else {}
            except (ValueError, TypeError):
                details = {}
            if not isinstance(details, dict):
                details = {}
            regions = details.get("text_regions", [])
            if not isinstance(regions, list) or len(regions) > 64 or any(not valid_region(r) for r in regions):
                item["blocking"].append("invalid_text_regions")
                item["text_region_count"] = None
            else:
                item["text_region_count"] = len(regions)
                clues = sorted(set(media["post"].get("tag_string_general", "").split()) & TEXT_CLUES)
                if not regions and clues:
                    item["review"].append("inspect_current_crop_for_text: " + ", ".join(clues))
            special = details.get("special_elements", [])
            visual_regions = details.get("visual_regions", [])
            if not isinstance(visual_regions, list) or len(visual_regions) > 64 or any(not valid_visual_region(r) for r in visual_regions):
                item["blocking"].append("invalid_visual_regions")
            item["visual_region_count"] = len(visual_regions) if isinstance(visual_regions, list) else None
            if details.get("image_sha256") and details["image_sha256"] != media["sha256"]:
                item["review"].append("annotation_regions_refer_to_other_image")
            if details.get("review_reasons"):
                item["review"].append("manual_review_required")
            if (not isinstance(special, list) or len(special) > 2
                    or any(not isinstance(e, str) or e not in {"x_ray", "cross_section"} for e in special)
                    or len(set(e for e in special if isinstance(e, str))) != len(special)):
                item["blocking"].append("invalid_special_elements")
            if include_details:
                item["existing_annotation_details"] = details
                item["current_text_artifacts"] = by_kind
            item["format_passed"] = not item["blocking"]
            images.append(item)
        return {"scope_ids": ids, "checked": len(images),
                "format_blocked": sum(bool(i["blocking"]) for i in images),
                "needs_visual_review": sum(bool(i["review"]) for i in images),
                "read_only": True, "semantic_approval": False, "images": images}
    finally:
        connection.close()


def self_test():
    import tempfile
    with tempfile.TemporaryDirectory(prefix="booru-annotation-audit-") as directory:
        directory = Path(directory)
        db = directory / "catalog.sqlite3"
        rules = {"aliases": {}, "categories": {}, "characters": [{"base": "base", "variant": "variant_form", "verified": True}]}
        post = {"tag_string_artist": "test_artist", "tag_string_character": "base variant_form other_character",
                "tag_string": "base variant_form other_character test_artist scenery", "tag_string_general": "scenery artist_name"}
        media = {"path": str(directory / "original.png"), "post": post, "sha256": "current-test-hash"}
        region = {"text": "unreadable text", "bbox_1000_xyxy": [600, 900, 800, 990], "kind": "watermark", "legibility": "unreadable"}
        prefix = "@test artist, variant form, other character, "
        with closing(sqlite3.connect(db)) as c:
            c.executescript("CREATE TABLE media(id INTEGER,metadata TEXT); CREATE TABLE tag_rules(id INTEGER,metadata TEXT);"
                            "CREATE TABLE text_artifacts(media_id INTEGER,kind TEXT,content TEXT,sha256 TEXT,state TEXT,is_current INTEGER);"
                            "CREATE TABLE media_quality(media_id INTEGER,provenance TEXT);")
            c.execute("INSERT INTO media VALUES(1,?)", (json.dumps(media),))
            c.execute("INSERT INTO tag_rules VALUES(1,?)", (json.dumps(rules),))
            c.execute("INSERT INTO media_quality VALUES(1,?)", (json.dumps({"text_regions": [region], "special_elements": ["x_ray"]}),))
            def put(kind, text):
                c.execute("DELETE FROM text_artifacts WHERE kind=?", (kind,))
                c.execute("INSERT INTO text_artifacts VALUES(1,?,?,?,'complete',1)", (kind, text, hashlib.sha256(text.encode()).hexdigest()))
                (directory / ("original.tags.txt" if kind == "tags" else "original.txt")).write_bytes(text.encode())
                c.commit()
            put("caption", prefix + "A figure stands beside a tree in soft light.")
            put("tags", prefix + "scenery")
            before = hashlib.sha256(db.read_bytes()).hexdigest()
            assert audit(db, [1])["format_blocked"] == 0
            assert audit(db, [1])["needs_visual_review"] == 0
            assert hashlib.sha256(db.read_bytes()).hexdigest() == before
            put("tags", "highres, " + prefix + "scenery")
            issues = audit(db, [1])["images"][0]["blocking"]
            assert "tags_artist_prefix_or_order" in issues and any("non_visual_tags" in i for i in issues)
            put("tags", prefix + "scenery")
            c.execute("UPDATE media_quality SET provenance=?", (json.dumps({"text_regions": [], "special_elements": []}),)); c.commit()
            assert audit(db, [1])["format_blocked"] == 0 and audit(db, [1])["needs_visual_review"] == 1
            put("caption", prefix + "1girl, solo, standing")
            assert "caption_body_starts_as_tag_list" in audit(db, [1])["images"][0]["blocking"]
            assert audit(db, [99])["format_blocked"] == 1
            assert audit(db, [1], True)["images"][0]["existing_annotation_details"]["text_regions"] == []
            assert audit(db, [1], True)["images"][0]["current_text_artifacts"]["tags"] == prefix + "scenery"
            cyclic = {"characters": [{"base": "base", "variant": "variant_form", "verified": True},
                                     {"base": "variant_form", "variant": "base", "verified": True}]}
            assert "base" in identity_prefix(post, cyclic)[1]
            assert not valid_region(dict(region, bbox_1000_xyxy=[900, 900, 100, 990]))
            assert not valid_region(dict(region, text="guessed words"))
            assert valid_region(dict(region, text="SPR", legibility="partial", confidence=0.6))
            assert valid_region(dict(region, legibility="decorative"))
            assert not valid_region(dict(region, confidence=1.1))
            assert valid_visual_region({"kind":"detail_inset", "bbox_1000_xyxy":[500,500,900,900], "description":"A flower.", "relation":"Separate frame at lower right.", "confidence":None})
        missing = directory / "missing.sqlite3"
        try:
            audit(missing, [1])
            raise AssertionError("Missing DB should not open")
        except FileNotFoundError:
            assert not missing.exists()
    print("Self-test passed: read-only inspection, identity hierarchy, pollution detection, OCR review and malformed input.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path)
    parser.add_argument("--ids", nargs="+", type=int)
    parser.add_argument("--details", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        if args.catalog is None or args.ids is None:
            parser.error("--catalog and --ids are required")
        report = audit(args.catalog, args.ids, args.details)
        print(json.dumps(report, ensure_ascii=False, indent=2))
        raise SystemExit(2 if report["format_blocked"] else 0)
