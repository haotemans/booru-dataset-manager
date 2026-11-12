# PixelLoop-DiT pixel-art collection

## Goal
**Daily intake targets:** 10,000 new distinct static images and 1,000 new
GIF animations, each Beijing calendar day (UTC+8). This is **not** an all-time
cap. New accepted media continue to accumulate across days in the same private
dataset, with cross-day deduplication. These quantities are capacity targets,
not guarantees or proof of rights clearance.

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
- Lightweight state: datasets/pixelloop/progress.json (all-time accepted IDs,
  checksums, perceptual fingerprints, separate search cursors and per-day counts).
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
      --images-per-run 500 --gifs-per-run 50 --max-batch-mib 768

Repeat using the same directory to continue from the saved index.

## Continuous collection
1. Create a PRIVATE ModelScope dataset in your account; check rights and
   available storage before uploading any third-party media.
2. Configure GitHub Actions variable MODELSCOPE_DATASET_ID = owner/dataset.
3. Configure GitHub Actions secret MODELSCOPE_TOKEN (never in chat or commits).
4. Merge the workflow onto the default branch; GitHub schedule only runs there.
5. Scheduled hourly at minute 17 UTC (24 opportunities/day); each run may
   accept up to 500 images and 50 GIFs, with a combined 768 MiB cap and a
   100-minute job timeout. The **daily** acceptance ceiling is 10,000 images
   and 1,000 GIFs. A day is defined by Asia/Shanghai (UTC+8).
   Scheduled workflows may be delayed or skipped by GitHub; capacity caps
   do not constitute a delivery SLA.
6. The publisher uploads and readback-verifies the tar archive before recording
   progress. Without durable storage configured, scheduled jobs are skipped.
   Manual/branch test runs can produce an ephemeral artifact without advancing
   progress; failures do not count toward the target.

Each successful shard is uploaded privately and checked by remote SHA256
readback before the light index is pushed back to GitHub. Incomplete runs do
not advance the persistent checkpoint. The Actions artifact is limited to
reports and indexes, not full media archives. No daily intake is guaranteed;
source supply, network limits, GitHub runner minutes, storage quotas,
provider restrictions and human review may constrain completion.

A theoretical maximum of 24 batches × 768 MiB equals 18 GiB/day of
accepted original-media archives (before provider overhead). Real storage
usage depends on available media and format. Check repository, Actions
and ModelScope quotas regularly. Training rights and captions are separately
reviewed: acceptance is **not** clearance for model training.

## Training handoff
Keep original-resolution media and prepare separate nearest-neighbor 512-pixel
training transformations. Sample GIFs using timestamps at 8 FPS and at most
five-second windows with masks where frames are missing. Audit and review
captions, preserve author/source attribution, and keep near duplicates in
the same train/validation partition to avoid leakage.
