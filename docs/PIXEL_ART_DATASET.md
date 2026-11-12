# PixelLoop-DiT pixel-art collection

## Goal
Acquire and preliminarily curate 10,000 distinct static pixel-art images plus
1,000 distinct animated GIFs from Danbooru. This number is a TARGET, not a
claim of collection or licensing.

Danbooru searches use pixel_art rating:g (plus filetype:gif for animations).
Selection requires the pixel_art tag, general rating, minimum score/favorites,
short side at least 128 px, reasonable aspect ratio and file size. Each file
must decode successfully and match the Danbooru MD5. Static candidates that
appear blank or animated are rejected. GIF candidates must have 6–600 frames,
0.6–60 seconds duration, and meaningful changes across sampled frames.
A conservative 64-bit difference hash plus 8x8 quantized colors and a matching
aspect ratio screen for near duplicates. Every original keeps its source JSON.

Quality and copyright are NOT fully verified: human review is necessary for
pixel-art fidelity, aesthetics, age/content suitability, source attribution,
training rights and caption accuracy. A general-rated or publicly hosted post
is not permission to train or redistribute. The natural-language .txt is an
unverified heuristic draft that requires review before training.

## Persistent progress and files
- Lightweight state: datasets/pixelloop/progress.json (accepted IDs,
  checksums, perceptual fingerprints, separate search cursors).
- Per run: data/pixelloop-curated/batches/RUN_ID/{image,gif}/
  with original media and matching .txt and .json.
- Shard: data/pixelloop-curated/archives/pixelloop-RUN_ID.tar with SHA256.
- Report: data/pixelloop-curated/report.json

The index is checkpointed into Git only AFTER the complete archive is
uploaded to private ModelScope and downloaded back with matching SHA256.
Keep persistent original media and earlier shards. The seven-day GitHub Actions
artifact is a temporary review copy, not long-term storage.

## Local test and incremental download
    python -m pip install 'Pillow>=10,<13'
    python -m unittest discover -s tests -p test_pixel_art_collect.py -v
    python scripts/pixel_art_collect.py --root data/pixelloop-curated \
      --images-per-run 80 --gifs-per-run 8 --max-batch-mib 300

Repeat using the same directory to continue from the saved index.

## Continuous collection
1. Create a PRIVATE ModelScope dataset in your account; check rights and
   available storage before uploading any third-party media.
2. Configure GitHub Actions variable MODELSCOPE_DATASET_ID = owner/dataset.
3. Configure GitHub Actions secret MODELSCOPE_TOKEN (never in chat or commits).
4. Merge the workflow onto the default branch; GitHub schedule only runs there.
5. Daily 03:25 UTC runs try up to 80 images and 8 GIFs, with a 300 MiB cap.
6. The publisher uploads and readback-verifies the tar archive before recording
   progress. Without durable storage configured, scheduled jobs are skipped.
   Manual/branch test runs can produce an ephemeral artifact without advancing
   progress; failures do not count toward the target.

No time-to-target is guaranteed; source availability, API limits, provider
restrictions, storage quotas and human review can constrain progress.

## Training handoff
Keep original-resolution media and prepare separate nearest-neighbor 512-pixel
training transformations. Sample GIFs using timestamps at 8 FPS and at most
five-second windows with masks where frames are missing. Audit and review
captions, preserve author/source attribution, and keep near duplicates in
the same train/validation partition to avoid leakage.
