# Automatic training crops and native-size export

The review ratio buttons define the source selection. Crop confirmation uses
an automatic native-pixel plan: keep the selection at its original scale and
align both output dimensions to 32 by trimming edges. There is no area ceiling
and no automatic downsampling, including selections larger than the 2048 tier.
There is no manual resolution selector. The toolbar shows an approximate
768/1024/1536/2048 tier, determined by the nearest equivalent-area resolution
`sqrt(width * height)`, and the save button shows the actual pixel dimensions.
The tier label never triggers resizing by itself.

`bdm_domain::training` follows the local sd-scripts
`BucketManager(no_upscale=True, reso_steps=32)` geometry, including integer
rounding and centered edge trimming. Images are never enlarged. The selection
and resulting image must have a short side of at least 576 and at least
550,000 pixels. All confirmed crop/repair outputs and optional resolution
variants are saved as lossless PNG with a matching `.png` filename; source
post extensions and original bytes remain unchanged. Source selection, resized
dimensions, trim offsets, final dimensions and native-pixel mode are
recorded with the edit. A nominal aspect label can differ slightly from the
final rounded bucket ratio.

Retention is one complete original download and one current edited PNG, plus
one disposable preview thumbnail. Before an edit, an unchanged download needs
only one image file. Imports keep a separate `downloaded-original.<ext>` only
when transparency processing changes the working bytes. Edits preserve one
hash-verified original archive and replace the current PNG; they do not create
undo images or retain successive crop/inpainting versions. Crops and source
URLs remain in metadata. The review action **Restore complete original** returns
to the full verified download, rather than to a historical edited version.
Re-download likewise replaces the working result without an extra backup image.
Legacy files without a verified original are never silently called restored.

## Default: native-size flat export

All approved images with the selected successful text artifact are eligible,
regardless of their resolution tier. Each current image is copied byte for
byte, alongside its matching TXT, into the same directory. Export performs no
resizing, upscaling, tier filtering or tier-based folder splitting. Manually
modified images are protected, and existing text is preserved.

Re-exporting into the same managed directory updates the owned current image,
including a restored larger PNG with the same filename. Existing TXT files are
kept byte for byte, including their encoding and line endings; only missing TXT
files are created. This applies to flat, folder and multi-resolution layouts.
Ownership is checked against the prior image/source/hash receipt before replacing
an image. Manual image changes and unowned format collisions stop that item.
Old image formats are removed after the new image is verified. No image `.bak`
files are created, and matching hash-verified legacy export backups are removed.
A temporary metadata intent allows interrupted updates to resume without an
extra permanent image copy.

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

## Historical JPEG remediation

The explicit offline `--repair-crops-png <private-backup-dir> --catalog <absolute-db>`
command audits legacy JPEG crop records. Adding `--apply` first backs up the
SQLite catalog and each replaced JPEG. Only direct crops with a hash-verified
downloaded original, matching parent snapshot/bytes and an unchanged saved crop
geometry are rebuilt. The PNG is regenerated from that original, never from the
already recompressed JPEG. Unverifiable history is reported and left untouched.
Re-export migrates an owned JPEG/WebP pair to PNG after image hash and source
checks. Existing text is preserved; the verified new image replaces the old
format without retaining an image backup.

For an explicit verified-original rebuild, add `--native-originals <directory>` to
the offline repair command. Each `media_<id>/original.jpg` must match both the
source-post MD5 and the verified parent SHA-256. Retained original downloads or newly downloaded
originals with identical verified bytes are accepted. The old working PNG and catalog
are backed up; the original crop selection is regenerated at native scale,
never at its previous smaller area ceiling. Missing/invalid original files prevent
changes. Only 32-alignment edge trimming remains. Legacy saved crop plans without
`native=true` retain their old meaning when read; they are not silently relabeled.
