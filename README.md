# Booru Dataset Manager

A Windows desktop application for managing booru-sourced image datasets, visual identity, metadata sidecars, dataset export, and versioned ModelScope backups.

> **Project status:** Initial repository bootstrap. Features are implemented only when their code and tests exist; design specifications are not evidence of implementation. See [`docs/planning/IMPLEMENTATION_STATUS.md`](docs/planning/IMPLEMENTATION_STATUS.md).

## Planned architecture

- Rust workspace with an `egui`/`eframe` desktop application
- SQLite-backed metadata and migrations
- Managed original media with exact-hash duplicate detection
- DINOv3 embeddings and human-reviewable visual-identity matching
- Three UTF-8 sidecars per image: `*.raw_tags.txt`, `*.tags.txt`, and `*.txt`
- Independent artist, character, optimized-tag, and caption processing targets
- Dataset export profiles and resumable jobs
- Manual, versioned ModelScope dataset backups with SHA-256 verification

## Requirements and safety principles

- Original raw tags are provenance and must not be silently rewritten.
- Unknown or ambiguous domain-specific tags must be preserved for review, not guessed or generalized.
- Artist and character names are protected from AI rewriting.
- AI image upload is opt-in; API keys and session credentials must never be sent to AI providers or committed to source control.
- Source adapters must respect provider authentication requirements and access restrictions.
- A backup is considered current only after its complete manifest and all required objects have been verified.

## Development status

The implementation-status document distinguishes specified, implemented, and verified work. Build, platform integration, model inference, provider authentication, and remote-backup behavior must not be marked verified until the relevant checks have actually run.

## License

License selection is pending. Do not assume this project is licensed for redistribution until a license is added.