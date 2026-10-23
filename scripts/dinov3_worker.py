"""Local, pinned DINOv3 inference. JSON lines on stdout; no image network upload."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import urllib.request

REPO = "timm/vit_small_patch16_dinov3.lvd1689m"
REVISION = "3bf4720a82ec2066db88137180ff1f83a675cef0"
WEIGHTS_SHA256 = "2a1ec16ae28ffa07bc0ead0241ee7df9fc26451fe6f9f839b7b3afa0a906b040"
VERSION = f"{REPO}@{REVISION}:rgb224-bicubic-imagenet-cls-l2-v1"


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def download(root):
    root.mkdir(parents=True, exist_ok=True)
    base = f"https://huggingface.co/{REPO}/resolve/{REVISION}/"
    for name in ["config.json", "LICENSE.md", "model.safetensors"]:
        target = root / name
        if target.exists() and (name != "model.safetensors" or sha256(target) == WEIGHTS_SHA256):
            continue
        temporary = root / (name + ".partial")
        with urllib.request.urlopen(base + name, timeout=120) as response, temporary.open("wb") as stream:
            while block := response.read(1024 * 1024):
                stream.write(block)
            stream.flush()
            os.fsync(stream.fileno())
        if name == "model.safetensors" and sha256(temporary) != WEIGHTS_SHA256:
            raise ValueError("model_checksum")
        temporary.replace(target)
    (root / "model-version.json").write_text(json.dumps({"version": VERSION, "sha256": WEIGHTS_SHA256}), encoding="utf-8")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model-dir", type=Path, required=True)
    parser.add_argument("--download", action="store_true")
    args = parser.parse_args()
    if args.download:
        download(args.model_dir)
        print(json.dumps({"state": "ready", "version": VERSION}), flush=True)
        return
    weights = args.model_dir / "model.safetensors"
    if not weights.is_file() or sha256(weights) != WEIGHTS_SHA256:
        raise ValueError("model_missing_or_invalid")
    import torch
    import timm
    from PIL import Image
    from safetensors.torch import load_file
    from torchvision import transforms
    torch.set_num_threads(max(1, min(4, (os.cpu_count() or 2) // 2)))
    model = timm.create_model("vit_small_patch16_dinov3", pretrained=False, num_classes=0)
    model.load_state_dict(load_file(str(weights)))
    model.eval()
    device = "cpu"
    model.to(device)
    preprocess = transforms.Compose([
        transforms.Resize((224, 224), interpolation=transforms.InterpolationMode.BICUBIC),
        transforms.ToTensor(),
        transforms.Normalize((0.485, 0.456, 0.406), (0.229, 0.224, 0.225)),
    ])
    for line in sys.stdin:
        try:
            request = json.loads(line)
            with Image.open(request["path"]) as image:
                value = preprocess(image.convert("RGB")).unsqueeze(0)
            with torch.inference_mode():
                vector = model(value.to(device)).float().cpu().flatten()
            vector = torch.nn.functional.normalize(vector, dim=0)
            if vector.numel() != 384 or not torch.isfinite(vector).all():
                raise ValueError("embedding_invalid")
            print(json.dumps({"id": request["id"], "version": VERSION, "device": device, "vector": vector.tolist()}), flush=True)
        except Exception:
            print(json.dumps({"error": "local_inference_failed"}), flush=True)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        print(json.dumps({"error": "model_or_runtime_unavailable"}), flush=True)
        sys.exit(1)
