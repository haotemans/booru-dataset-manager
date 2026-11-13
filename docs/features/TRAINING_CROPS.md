# Automatic training crops and native-size export

The review ratio buttons define the source selection. Crop confirmation uses
an automatic plan: preserve available detail, align both output dimensions to
32, and apply the 2048-square area ceiling only when the selection exceeds it.
There is no manual resolution selector. The toolbar shows an approximate
768/1024/1536/2048 tier, determined by the nearest equivalent-area resolution
`sqrt(width * height)`, and the save button shows the actual pixel dimensions.
The tier label never triggers resizing by itself.

`bdm_domain::training` follows the local sd-scripts
`BucketManager(no_upscale=True, reso_steps=32)` geometry, including integer
rounding and centered edge trimming. Images are never enlarged. The selection
and resulting image must have a short side of at least 576 and at least
550,000 pixels. JPEG training output uses quality 95. Source selection, resized
dimensions, trim offsets, final dimensions and the 2048 area ceiling are
recorded with the edit. A nominal aspect label can differ slightly from the
final rounded bucket ratio.

New imports retain `downloaded-original.<ext>` before transparency processing.
Edits retain a hash-verified source archive; the working image and thumbnail
can change while this archive stays intact. Legacy files without a download
archive retain their first available input snapshot. Metadata identifies
whether its bytes match the source MD5. Source metadata and URLs are preserved.
Restore-previous-version retains the preceding edit geometry. Existing crops
made with a lower ceiling are not enlarged or rewritten automatically.

## Default: native-size flat export

All approved images with the selected successful text artifact are eligible,
regardless of their resolution tier. Each current image is copied byte for
byte, alongside its matching TXT, into the same directory. Export performs no
resizing, upscaling, tier filtering or tier-based folder splitting. Existing
images and manually edited text retain integrity protection.

`dataset.toml` is a **1024 training reference**, with `enable_bucket=true`,
`bucket_no_upscale=true`, `bucket_reso_steps=32`, and random crop/flip/color
augmentation disabled. The trainer may resize or trim larger/unaligned inputs
when it loads this native-size export; exporting itself does not. This replaces
the former behavior that inferred the training resolution from the largest
crop ceiling and rejected mixed native dimensions. The local Anima/Moss entry
points use the managed reference when given its export directory.

## Optional: multiple resolution copies

An explicit, default-off export option generates separate `768`, `1024`,
`1536` and `2048` directories. Each contains flat image/TXT pairs and its own
matching `dataset.toml`. Variants are derived from the current image without
modifying it. Small images are not enlarged: an image can have identical
sizes in several larger-ceiling directories. A variant that cannot meet the
576-short-side/550,000-pixel floor is omitted; an image with no valid variants
fails the task with an explicit reason. Each generated variant is checked to
remain geometrically unchanged by the corresponding training configuration.

The configs use absolute export paths. Export again after moving the dataset.
Provenance is under `.bdm-manifests`; manual config changes are preserved and
reported as integrity conflicts. Cropping does not prove that existing text
still describes the cropped content. Review captions before training; neither
cropping nor exporting invokes an API or regenerates descriptions.
