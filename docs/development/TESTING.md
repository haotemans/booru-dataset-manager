# Testing strategy

Unit tests: inclusive 800×800 gate, unknown-dimension policy, sidecar suffix/provenance, protected-term mapping, export filter composition, review transitions, deterministic manifests, SHA-256 checks and backup states.

Integration tests: temporary directories and SQLite databases; crash recovery across file/database steps; MD5 uniqueness; sidecar atomic replacement/history; export conflict/resume; backup completeness/restore integrity; fake-adapter retry, cancellation and partial-upload failures.

Platform/live tests: Windows build and UI smoke test; real named DINOv3 artifact compatibility/performance; supported provider auth; ModelScope private-repository upload/readback/hash/restore. Mock tests do not prove live integration. Never print credentials.

Required baseline commands (not yet run): cargo fmt --all -- --check; cargo test --workspace; cargo clippy --workspace --all-targets -- -D warnings.
