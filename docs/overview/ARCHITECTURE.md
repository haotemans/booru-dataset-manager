# Architecture

Dependency direction: desktop UI → application use cases → domain + ports; infrastructure implements ports; config holds non-secret settings. Domain must not depend on egui, SQLite, HTTP clients, or filesystem adapters.

SQLite is the metadata/index source of truth. Managed originals and UTF-8 sidecars are durable artifacts. Use temporary writes, atomic replacement where supported, and startup reconciliation for interrupted filesystem/database operations.

Ingestion order: source metadata → original-dimension gate → MD5 duplicate lookup → temporary download → actual image validation → SHA-256 integrity hash → managed-library move → database transaction → DINOv3 embedding and similarity search. If the model is unavailable, retain the image and mark embedding pending.

DINOv3 vectors are normalized and compared with cosine similarity. Similarity is a review signal, not proof; thresholds require labeled-data calibration. Keep file entity, visual identity and dataset classification distinct.

Never log credentials, send API keys/session tokens to AI, or upload images to AI without explicit opt-in. Backup latest pointer advances only after all required objects verify.
