import hashlib
import importlib.util
import io
import json
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path
from urllib.parse import parse_qs, urlparse

from PIL import Image, ImageDraw

src = Path(__file__).resolve().parents[1] / "scripts/pixel_art_collect.py"
spec = importlib.util.spec_from_file_location("pixel_art_collect", src)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


def image_bytes(seed=0, size=144):
    im = Image.new("RGB", (size, size), (20, 40, 70))
    draw = ImageDraw.Draw(im)
    for i in range(6):
        draw.rectangle((i * 20, i * 12, i * 20 + 40, i * 12 + 38),
                       fill=((70 + i * 20 + seed) % 255,
                             (20 + seed + i * 15) % 255, (90 + i * 20) % 255))
    out = io.BytesIO()
    im.save(out, "PNG")
    return out.getvalue()


def gif_bytes():
    frames = []
    for i in range(8):
        img = Image.new("RGB", (144, 144), (0, 20, 30))
        ImageDraw.Draw(img).rectangle((10 + i * 12, 45, 36 + i * 12, 70),
                                       fill=(255, 0, 0))
        frames.append(img)
    out = io.BytesIO()
    frames[0].save(out, "GIF", save_all=True, append_images=frames[1:],
                   duration=100, loop=0)
    return out.getvalue()


class FakeFetcher:
    def __init__(self, post, payload):
        self.post = post
        self.payload = payload
    def get(self, url, max_bytes):
        if "/posts.json?" in url:
            if parse_qs(urlparse(url).query).get("page"):
                return b"[]"
            return json.dumps([self.post]).encode("utf-8")
        return self.payload


class CollectorTests(unittest.TestCase):
    def test_accept_and_filter_metadata(self):
        p = dict(rating="g", tag_string="pixel_art scenery",
                 tag_string_general="pixel_art scenery", score=5, fav_count=0,
                 file_ext="png", md5="a" * 32, image_width=144, image_height=144)
        self.assertIsNone(mod.preflight(p, "image"))
        self.assertEqual(mod.preflight({**p, "rating": "e"}, "image"),
                         "not_general_or_deleted")
        self.assertEqual(mod.preflight({**p, "score": 0}, "image"),
                         "low_community_score")
        self.assertEqual(mod.preflight({**p, "tag_string": "pixel_art color_guide"}, "image"), "excluded_tag")
        self.assertEqual(mod.preflight({**p, "tag_string": "pixel_art screenshot"}, "image"),
                         "excluded_tag")

    def test_decode_and_motion(self):
        info = mod.media_info(image_bytes(), "image")
        self.assertEqual(info["frames"], 1)
        gif = mod.media_info(gif_bytes(), "gif")
        self.assertEqual(gif["frames"], 8)
        self.assertEqual(gif["duration_ms"], 800)
        self.assertGreaterEqual(gif["unique_sampled_frames"], 4)
        self.assertEqual(len(gif["motion_hashes"]), 5)
        self.assertEqual(mod.near_duplicates([{"id": 7, "kind": "gif", **gif}], {"kind": "gif", **gif}), "7")
        different = {"kind": "gif", **gif, "motion_hashes": ["ffffffffffffffff"] + gif["motion_hashes"][1:]}
        self.assertIsNone(mod.near_duplicates([{"id": 7, "kind": "gif", **gif}], different))
        with self.assertRaises(ValueError):
            mod.media_info(image_bytes(size=60), "image")
        with self.assertRaises(ValueError):
            mod.media_info(gif_bytes(), "image")

    def test_duplicate_similarity(self):
        info = mod.media_info(image_bytes(), "image")
        self.assertEqual(mod.near_duplicates([{"id": 10, "kind": "image", **info}],
                                             {"kind": "image", **info}), "10")
        self.assertIsNone(mod.near_duplicates([{"id": 10, "kind": "image", **info}],
                                              {"kind": "gif", **info}))

    def test_only_donmai_media_hosts(self):
        self.assertTrue(mod.allowed_media_url("https://cdn.donmai.us/original/test.png"))
        self.assertFalse(mod.allowed_media_url("https://donmai.us.attacker.example/x"))
        self.assertFalse(mod.allowed_media_url("http://cdn.donmai.us/x"))

    def test_persistent_resume(self):
        payload = image_bytes()
        post = dict(id=1000, rating="g", tag_string="pixel_art solo",
                    tag_string_general="pixel_art solo", score=5, fav_count=5,
                    file_ext="png", md5=hashlib.md5(payload).hexdigest(),
                    file_url="https://cdn.donmai.us/image.png",
                    image_width=144, image_height=144,
                    file_size=len(payload), is_deleted=False)
        with tempfile.TemporaryDirectory() as temp:
            opts = Namespace(root=temp, state_file=None, run_id="first",
                target_images=10000, target_gifs=1000, images_per_run=1,
                gifs_per_run=0, max_pages=1, max_batch_mib=1,
                min_short=128, min_score=4, delay=0)
            first = mod.run(opts, FakeFetcher(post, payload))
            self.assertEqual(first["new_images"], 1)
            self.assertEqual(first["cursor"]["image"], 1000)
            self.assertTrue((Path(temp) / "archives/pixelloop-first.tar").exists())
            opts.run_id = "second"
            second = mod.run(opts, FakeFetcher(post, payload))
            self.assertEqual(second["new_images"], 0)
            self.assertEqual(second["total_images"], 1)


if __name__ == "__main__":
    unittest.main()
