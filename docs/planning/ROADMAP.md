# Delivery roadmap and exit gates

## P0 — repository and build foundation

Workspace compiles; dependency boundaries enforced; CI runs format, tests and clippy. Record actual CI status. No feature may be called verified without a passing check or attached evidence.

## P1 — durable media core

Implement SQLite migrations, media/source entities, managed filesystem storage, atomic sidecar writes, orphan recovery, original-dimension gate and database-enforced MD5 duplicate prevention. Exit gate: integration tests cover interrupted download/import, duplicate races, unknown dimensions and database restart.

## P2 — metadata and review

Implement artist/character relations, raw/optimized/caption sidecars, artifact versions, protected-term registry, target-isolated AI jobs and human review. Exit gate: raw provenance immutability and target isolation tests pass; reviewed artifacts cannot be silently overwritten.

## P3 — DINOv3 identity

Validate a named model artifact and license, preprocessing contract, embedding dimensions and runtime compatibility. Implement normalized embeddings, candidate retrieval and reviewable merge/split. Exit gate: labeled evaluation set reports precision/recall and latency; thresholds are versioned and not hard-coded without evidence.

## P4 — providers and ingestion queue

Implement providers one at a time behind a common port. Exit gate per provider: pagination, rate limit, auth expiry, cancellation, original-file selection and live smoke tests pass.

## P5 — export and dashboard

Implement shared export service, immutable job snapshots, conflict policies, resumability and stats queries. Exit gate: export manifest matches copied files and sidecars; resumed jobs do not duplicate or corrupt output.

## P6 — backup and restore

Validate actual ModelScope interface limits first; implement private-repository upload, immutable version manifests, SHA-256 remote verification and latest-pointer commit-last semantics. Exit gate: restore into an empty destination and verify image hashes, SQLite integrity and associations; simulate partial upload and local deletion.

## P7 — Windows release hardening

Build and smoke-test on supported Windows versions, verify paths containing Unicode/spaces, filesystem replacement semantics, credential-store behavior, installer/update and crash recovery. Do not use Linux CI as evidence of Windows correctness.

Features can be reordered, but each phase must retain its exit gate. Model weights, provider credentials and external account access are test prerequisites, not implied deliverables.
