"""Offline tests for read-only artist dataset preflight."""
import importlib.util
import sys
import types
import unittest
from pathlib import Path

path = Path(__file__).resolve().parents[1] / "scripts/artist_dataset_preflight.py"
spec = importlib.util.spec_from_file_location("artist_dataset_preflight", path)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


def post(**kwargs):
    item = {
        "id": 123, "rating": "g", "is_deleted": False,
        "file_ext": "png", "image_width": 1536, "image_height": 1536,
        "created_at": "2026-05-05T10:00:00Z", "tag_string_artist": "rella",
    }
    item.update(kwargs)
    return item


class Tests(unittest.TestCase):
    def test_1536_preserved(self):
        status, info = mod.evaluate(post())
        self.assertEqual(status, "candidate")
        self.assertEqual(info["planned_stored_size"], [1536, 1536])

    def test_high_resolution_downsampled_without_enlargement(self):
        status, info = mod.evaluate(post(image_width=4096, image_height=3072))
        self.assertEqual(status, "candidate")
        self.assertEqual(info["planned_stored_size"], [2048, 1536])

    def test_resolution_limits(self):
        self.assertEqual(mod.evaluate(post(image_width=512, image_height=1024))[0], "small_resolution")
        self.assertEqual(mod.evaluate(post(image_width=576, image_height=960))[0], "candidate")
        self.assertEqual(mod.evaluate(post(image_width=768, image_height=768))[0], "candidate")

    def test_rating_and_format_filter(self):
        self.assertEqual(mod.evaluate(post(rating="e"))[0], "restricted_or_deleted")
        self.assertEqual(mod.evaluate(post(file_ext="gif"))[0], "unsupported_format")

    def test_year_filter_upload_time_only(self):
        self.assertEqual(mod.evaluate(post(created_at="2019-01-01T00:00:00Z"))[0],
                         "outside_upload_date_window")
        self.assertEqual(mod.evaluate(post(created_at=None))[0], "unknown_upload_date")
        self.assertEqual(mod.evaluate(post())[1]["year_confidence"], "upload_date_only")

    def test_public_dataset_not_accepted(self):
        class FakeApi:
            def __init__(self, token):
                pass
            def get_repo(self, dataset_id, repo_type):
                return {"visibility": "public"}
            def list_repo_files(self, dataset_id, repo_type):
                return ["README.md"]
        prev = sys.modules.get("modelscope_hub")
        sys.modules["modelscope_hub"] = types.SimpleNamespace(HubApi=FakeApi)
        try:
            with self.assertRaisesRegex(ValueError, "private"):
                mod.inspect_dataset("user/dataset", "dummy-secret")
        finally:
            if prev is None:
                del sys.modules["modelscope_hub"]
            else:
                sys.modules["modelscope_hub"] = prev


if __name__ == "__main__":
    unittest.main()
