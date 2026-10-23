# Local inpainting

The review editor uses Anime LaMa by default for manga/anime erasure, with generic LaMa as an optional model. No image is uploaded. This is local removal/filling without a text prompt; generative repainting and AI region detection are not implemented.

Run `scripts/setup_analysis.ps1` if the local PyTorch runtime is missing. The editor's Install button downloads only the selected weights. Generation never implicitly downloads or changes models. Cancel terminates the local worker.

Anime LaMa is the TorchScript export of dreMaz's AnimeMangaInpainting checkpoint, pinned to [TareHimself revision b592884](https://huggingface.co/TareHimself/AnimeMangaInpainting-torchscript/tree/b592884b2b6589e57df6916db6bcac7a6307fc58). `anime-lama.pt` must have SHA-256 `95d4bf7eb2f13729351ac0a0693c9b9b469b264fc1542ca3fa551b59b464a160`; download size 204,190,262 bytes. The checkpoint/export is MIT; the upstream LaMa architecture remains Apache-2.0. See the repository's LICENSE and NOTICE. Generic LaMa uses the IOPaint-hosted big-lama TorchScript artifact with its published MD5 checksum.

The worker uses CPU, including when the installed PyTorch supports CUDA. It uses a context crop with 128 source pixels of padding, bounded to 1024 for inference. Results retain source dimensions, alpha and unmasked pixels in the lossless preview. JPEG commits re-encode in JPEG and therefore are not pixel-lossless outside the repaired area. Source format is retained by default.

Weights are ignored by Git. The editor records model revision/checksum, crop settings, source/parent/output hashes, mask, selection coordinates and timestamp when the user confirms replacement. Only the current image and current mask remain locally; temporary previews are disposable.
