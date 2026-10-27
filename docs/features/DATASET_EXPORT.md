# Dataset export

Artist page, character page and global center use one ExportService. Scope presets: current artist, current character, artist plus character, multiple artists, multiple characters, custom filters, whole library. Multi-select within one category defaults OR; artist plus character defaults AND. Character export defaults to linked media, with options to exclude media linked to other characters or include all associations.

Filters: review state, rating, resolution, source, tags, missing text and DINO variants. Default includes only Keep; pending, unsatisfactory and needs-review are excluded unless configured. Representative-only export uses the automatically selected most-aesthetically-appealing image per eligible source-confirmed difference set by default; users can manually override the choice, and all variants remain available through an explicit all-variants export option. Broad association groups must not be collapsed to one image unless the selected relation set is explicitly marked interchangeable for that export purpose.

Layouts: artist/character folders; nested artist/character; or images/, captions/, tags/, raw_tags/, manifest.jsonl. Conflict choices: skip, overwrite, auto-rename, cancel. Preview counts, exclusions, disk estimate and conflicts. Profiles reusable; job snapshot immutable, resumable and logged. Hardlinks are same-volume dependent and can expose in-place mutation risks. Manifest includes media ID/path/hash, sidecars, associations, source and review state.

## Training-ready flat export and verified incremental refresh

New export jobs default to `flat` layout: `media_<id>.png|jpg|webp` alongside exactly `media_<id>.txt` in the selected destination. The selected Tag or Caption artifact determines the TXT content. Per-item provenance and integrity metadata are stored in hidden `.bdm-manifests/media_<id>.json` files rather than cluttering the training root. The UI can explicitly switch to the original one-folder-per-image layout; old jobs without a layout snapshot retain that legacy behavior.

Rerunning a flat export checks the existing image hash and skips unchanged file pairs. If only an automatically exported TXT changed, it is atomically replaced **only** when the prior manifest proves the existing text and image match the previous exported version. A manual modification, unknown pre-existing text, or image hash mismatch yields an integrity error rather than overwriting the user's data. The task keeps its persisted cursor for bounded resume.
