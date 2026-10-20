# Dataset export

Artist page, character page and global center use one ExportService. Scope presets: current artist, current character, artist plus character, multiple artists, multiple characters, custom filters, whole library. Multi-select within one category defaults OR; artist plus character defaults AND. Character export defaults to linked media, with options to exclude media linked to other characters or include all associations.

Filters: review state, rating, resolution, source, tags, missing text and DINO variants. Default includes only Keep; pending, unsatisfactory and needs-review are excluded unless configured. Default DINO export uses representative media; all variants are configurable.

Layouts: artist/character folders; nested artist/character; or images/, captions/, tags/, raw_tags/, manifest.jsonl. Conflict choices: skip, overwrite, auto-rename, cancel. Preview counts, exclusions, disk estimate and conflicts. Profiles reusable; job snapshot immutable, resumable and logged. Hardlinks are same-volume dependent and can expose in-place mutation risks. Manifest includes media ID/path/hash, sidecars, associations, source and review state.
