# Model assets

The development model lives in `models/dinov3/`: `model.safetensors`, `config.json`, `model-version.json` and upstream `LICENSE.md`. The desktop default and opt-in local inference test use this directory. Explicit model-path settings take precedence. The Python runtime remains in `target/analysis-runtime/`.

The current DINOv3 ViT-S/16 is the small model (about 21.6 million parameters, 384-dimensional features); its FP32 weights are 86,362,376 bytes (86.4 MB / 82.4 MiB). It is a complete feature extractor, not a full generative model. Larger DINOv3 models require an explicit model/preprocessing compatibility change and re-embedding; swapping weights alone is unsupported.

Model weights are not committed. DINOv3 integration requires a verified model artifact, license review, preprocessing/version metadata, and inference-backend compatibility tests. Do not claim inference works until an actual model has been run.

The development adapter now runs pinned `timm/vit_small_patch16_dinov3.lvd1689m` with 384-dimensional L2-normalized output on CPU, including when the installed PyTorch supports CUDA. Model download verifies its published SHA-256 and preserves upstream `LICENSE.md`; details and actual CPU smoke evidence are in [workflow delivery](../docs/planning/WORKFLOW_DELIVERY.md). `scripts/setup_analysis.ps1` prepares the isolated Python runtime. Runtime bundling remains pending.
