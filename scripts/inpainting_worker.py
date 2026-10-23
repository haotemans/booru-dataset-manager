"""Local LaMa inference; no image uploads, no implicit model downloads."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import time
import urllib.request

MODELS = {
    "anime-lama": ("https://huggingface.co/TareHimself/AnimeMangaInpainting-torchscript/resolve/b592884b2b6589e57df6916db6bcac7a6307fc58/anime_manga_lama.pt", "95d4bf7eb2f13729351ac0a0693c9b9b469b264fc1542ca3fa551b59b464a160"),
    "lama": ("https://github.com/Sanster/models/releases/download/add_big_lama/big-lama.pt", "e3aa4aaa15225a33ec84f9f4bc47e500"),
}

def checksum(path, expected):
    digest = hashlib.sha256() if len(expected) == 64 else hashlib.md5()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

class Inpainter:
    def __init__(self, weights, expected):
        if not weights.is_file() or checksum(weights, expected) != expected:
            raise ValueError("Install the selected model first")
        import torch
        torch.set_num_threads(max(1, min(4, (os.cpu_count() or 2) // 2)))
        self.device = "cpu"
        self.model = torch.jit.load(str(weights), map_location=self.device).eval()

    def infer(self, image_path, mask_path, output_path):
        import numpy as np
        import torch
        from PIL import Image
        source = Image.open(image_path).convert("RGB")
        mask = Image.open(mask_path).convert("L")
        if source.size != mask.size or not mask.getbbox():
            raise ValueError("Empty or incompatible mask")
        image = np.asarray(source).copy()
        selection = np.asarray(mask) > 0
        # Work on a bounded context crop; leave every unselected pixel untouched.
        left, top, right, bottom = mask.getbbox()
        left, top = max(0, left - 128), max(0, top - 128)
        right, bottom = min(source.width, right + 128), min(source.height, bottom + 128)
        crop = Image.fromarray(image[top:bottom, left:right])
        crop_mask = mask.crop((left, top, right, bottom))
        original_size = crop.size
        scale = min(1.0, 1024 / max(crop.size))
        if scale < 1:
            size = tuple(max(1, round(v * scale)) for v in crop.size)
            crop = crop.resize(size, Image.Resampling.LANCZOS)
            crop_mask = crop_mask.resize(size, Image.Resampling.NEAREST)
        rgb = np.asarray(crop).copy()
        binary = (np.asarray(crop_mask) > 0).astype(np.float32)
        h, w = binary.shape
        pad = ((0, (-h) % 8), (0, (-w) % 8))
        rgb = np.pad(rgb, (*pad, (0, 0)), mode="edge")
        binary = np.pad(binary, pad, mode="constant")
        with torch.inference_mode():
            result = self.model(torch.from_numpy(rgb.transpose(2, 0, 1)).float()[None].to(self.device) / 255,
                                torch.from_numpy(binary)[None, None].to(self.device))
        result = np.clip(result[0].permute(1, 2, 0).cpu().numpy()[:h, :w] * 255, 0, 255).astype(np.uint8)
        result = np.asarray(Image.fromarray(result).resize(original_size, Image.Resampling.LANCZOS))
        area = image[top:bottom, left:right]
        area[selection[top:bottom, left:right]] = result[selection[top:bottom, left:right]]
        Image.fromarray(image).save(output_path, format="PNG", compress_level=1)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", choices=MODELS, required=True)
    parser.add_argument("--model-dir", type=Path, required=True)
    parser.add_argument("--download", action="store_true")
    parser.add_argument("--stream", action="store_true")
    parser.add_argument("--image", type=Path)
    parser.add_argument("--mask", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    url, expected = MODELS[args.model]
    weights = args.model_dir / (args.model + ".pt")
    if args.download:
        args.model_dir.mkdir(parents=True, exist_ok=True)
        if not weights.is_file() or checksum(weights, expected) != expected:
            partial = weights.with_suffix(".partial")
            try:
                with urllib.request.urlopen(url, timeout=120) as response, partial.open("wb") as target:
                    for chunk in iter(lambda: response.read(1024 * 1024), b""):
                        target.write(chunk)
                    target.flush()
                    os.fsync(target.fileno())
                if checksum(partial, expected) != expected:
                    raise ValueError("Model checksum mismatch")
                partial.replace(weights)
            finally:
                partial.unlink(missing_ok=True)
        print("Model ready", flush=True)
        return
    if args.stream:
        # Lazy initialization makes startup/model-load failures request-scoped too.
        inpainter = None
        for line in sys.stdin:
            request = {}
            started = time.perf_counter()
            try:
                request = json.loads(line)
                if inpainter is None:
                    inpainter = Inpainter(weights, expected)
                inpainter.infer(request["image"], request["mask"], request["output"])
                response = {"id": request["id"], "device": inpainter.device,
                            "elapsed_ms": round((time.perf_counter() - started) * 1000)}
            except Exception as error:
                response = {"id": request.get("id"), "error": str(error)[-900:]}
            print(json.dumps(response), flush=True)
    else:
        Inpainter(weights, expected).infer(args.image, args.mask, args.output)

if __name__ == "__main__":
    main()
