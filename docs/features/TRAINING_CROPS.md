# Training crops and sd-scripts compatibility

The review ratio buttons define the source selection. Crop confirmation now
prepares a training image, using a persistent 768/1024/1536/2048 area-budget
selector (default 1024). Both output dimensions are multiples of 32. The
selected budget is an upper bound, not a claim that small inputs have acquired
more detail. The confirmation button displays the actual output dimensions.

`bdm_domain::training` follows the local sd-scripts
`BucketManager(no_upscale=True, reso_steps=32)` geometry, including integer
rounding and centered edge trimming. It rejects upscaling and verifies that
running the output dimensions through the same algorithm again makes no
geometrical change. The input selection and output must meet short side 576
and 550,000 pixels. Unsupported small/wide selections remain unsaved with a
reason; images are never enlarged to satisfy those limits.

The image pipeline crops the user selection, downsamples with Lanczos3 when
needed, and trims the calculated edges. JPEG training output uses quality 95.
Source selection, resized dimensions, trim offsets, final dimensions and
resolution budget are recorded with the edit. A nominal aspect label need not
equal the final rounded bucket ratio exactly.

New imports retain `downloaded-original.<ext>` before transparency processing.
Edits retain a hash-verified source archive; the working image and thumbnail
can change while this archive stays intact. Legacy files without an original
download archive retain their first available input snapshot. Its metadata
states whether the bytes match the source MD5; an already altered legacy file
is not relabeled as a recovered original. Source metadata and source URLs are
preserved. Restore-previous-version retains the preceding edit geometry.

Flat export of training crops also writes `dataset.toml`, with the maximum
selected area budget, `enable_bucket=true`, `bucket_no_upscale=true`,
`bucket_reso_steps=32`, and random crop/flip/color augmentation disabled. It
checks every image already present in that export root before generating the
config. Misaligned mixed-in images make the export report a failure rather
than publishing an inaccurate training-ready config. Caption versus Tag
selection still determines the single matching TXT per exported image.

The generated config uses the absolute export directory. Export again after
moving the dataset. Its provenance is under `.bdm-manifests`; manual config
changes are preserved and reported as integrity conflicts instead of silently
overwritten. Training must use this config. The local Anima/Moss entry points
must preserve explicit TOML bucket settings; folder defaults are 32/no-upscale.

Cropping does not establish that an existing caption still describes the
cropped content. Review text before training; this action does not invoke an
API or rewrite text automatically.
