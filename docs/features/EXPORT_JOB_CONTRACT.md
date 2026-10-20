# Export job contract

An export job is created from a validated immutable snapshot: scope/filter expression, entity IDs, review policy, sidecar selection, image format, variant policy, layout, conflict policy and destination. Later profile edits do not mutate a running job.

## Planning phase

Resolve the media set, apply filters deterministically, identify missing originals/sidecars, resolve filename conflicts, estimate bytes and verify destination writability. Preview must distinguish eligible, excluded by filter, excluded by review policy, missing artifact, conflict and failed validation. Do not report an estimate as exact if compression/transcoding changes output size.

## Execution and resume

Write each output to a temporary destination and atomically promote after validation when possible. Record each item state and content hash. On resume, verify existing outputs against the expected hash/metadata before skipping; never trust file existence alone. If overwrite is selected, preserve an audit entry and ensure the replacement is complete before removing the old output. Cancellation leaves a resumable job with clear partial-output accounting.

## Manifest

Each JSONL record includes media ID, original SHA-256, exported relative path, output hash if transformed, source records, artist/character IDs and names, review status/rating, selected sidecar versions/hashes, visual identity group and export profile version. Paths must be relative and must not escape the export root. Serialize UTF-8 JSON Lines with stable field semantics and documented schema version.

## Acceptance tests

Filename collisions, Unicode names, path traversal attempts, missing sidecars, changed source file, overwrite failure, interrupted copy, hardlink unsupported, cross-volume hardlink, resume after partial export, and manifest-to-output reconciliation.
