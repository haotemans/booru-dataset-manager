# Implementation status

Status labels are strict: **Specified** means documented; **Implemented** means code exists but may be unverified; **Verified** means a named check actually passed with evidence.

| Area | Status | Evidence / limitation |
|---|---|---|
| Repository bootstrap | Implemented | README and Rust skeleton committed |
| Domain resolution gate | Implemented, not verified | Unit tests exist; CI has not run |
| Three sidecar type definitions | Implemented, not verified | Domain enum exists; lifecycle not implemented |
| SQLite persistence/migrations | Specified | No adapter yet |
| Provider ingestion/auth | Specified | No provider adapters yet |
| MD5 duplicate persistence | Specified | No database uniqueness constraint yet |
| DINOv3 inference/search | Specified | No model or inference adapter |
| AI optimization/review/export/dashboard | Specified | Workflows not implemented |
| ModelScope backup/restore | Specified | No remote adapter or restore test |
| Windows packaging | Specified | Not built/tested on Windows |

Required checks: cargo fmt --all -- --check; cargo test --workspace; cargo clippy --workspace --all-targets -- -D warnings; Windows smoke test; real DINOv3 evaluation; provider auth checks; ModelScope private-repo upload/readback/hash/restore. Do not mark verified without recorded evidence.
