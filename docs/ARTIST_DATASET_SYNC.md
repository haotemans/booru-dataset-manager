# Artist data: read-only ModelScope preflight

This is a separate workflow from the existing Pixel Art pipeline. It currently
**does not download, caption, crop, or upload any media**.

## Destination and secrets

Target: `feiyun12/artist_datasets` (shown in GitHub as
`MODELSCOPE_DATASET_ID2`). The workflow reads that **repository secret**
as `MODELSCOPE_DATASET_ID`, and reads `MODELSCOPE_TOKEN` as a secret.
Secrets must be configured in `haotemans/booru-dataset-manager`, not
another GitHub repository. The actual token value must never be committed.

The ModelScope destination must be **private** for future third-party
media processing. Read-only preflight rejects a non-private destination.
Verify the repo type, path, visibility and file listing before proceeding.

## How to check

1. Open **Actions → Artist dataset preflight → Run workflow** on the default
   branch, after the feature PR has been merged.
2. Verify its test job and inspection job succeed.
3. Open the **artist-dataset-preflight-report** artifact. It contains only
   metadata and candidate dimensions; never secrets or image bytes.
4. Check the existing ModelScope file layout for collisions.

A pull request only runs offline unit tests. The explicit manual workflow is
the only path that contacts ModelScope and Danbooru.

## Agreed future production contract (not yet implemented)

- Use the reviewed artist allowlist, rather than all found Artist Tags.
- Select artwork from 2020–2026, prioritizing recent works. Danbooru
  `created_at` is an *upload date* and not proof of an artwork's original year.
- Inspect 500 candidate images per analysis batch; scan 50 artists daily.
- Minimum short side 576px and 550,000 effective pixels; avoid any upscaling.
- Preserve high-resolution sources up to longest side 2048px; reduce larger
  images with quality resampling, protect the color profile, and retain source
  URLs and original hashes.
- Candidate ratio pool 11; normally 9 active, ceiling 13 if evidence warrants.
  Resolution and crop planning should preserve text, figures and composition,
  align stored dimensions to multiples of 32, and log decisions.
- Use source identity, exact hashes and reviewed perceptual similarity for
  duplicate removal.
- Style organization: **Artist Tag → year tag → style/lighting/material tags**;
  these groupings do not create new resolution buckets.
- Four files per accepted image: image, `ID.caption.txt`,
  `ID.tags.txt`, and immutable `ID.raw.txt`.
- Caption: coherent English description including artist, character,
  appearance, clothing, scene, composition, lighting, skin, brushwork,
  visible typography, overlays and accurately normalized 0–1000 BBoxes.
- Optimized Danbooru tags: `@artist`, character names and validated
  category tags, replace underscores with spaces, comma-separated.
- Raw TXT: unchanged original Danbooru post JSON, preserved for correction.
- Image captions require actual visual verification; tag translation alone
  is not an acceptable substitute.
- Media upload requires review of redistribution rights, content suitability,
  verified remote persistence and a resumable checkpoint. A private dataset
  does not by itself grant training or redistribution rights.

## Local offline checks

```sh
python -m unittest discover -s tests -p test_artist_dataset_preflight.py -v
```

The separate preflight script requires internet access and credentials for
the optional ModelScope check. The pilot list in
`datasets/artists/pilot_artists.json` is a small sample, not the full
approved artist roster.
