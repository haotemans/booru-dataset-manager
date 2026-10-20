# Security threat model

## Assets

Original media, raw source metadata, user-authored captions/tags, provider credentials, AI API keys, private backup repository, database and backup manifests, vision-upload consent records and derived crop recipes.

## Trust boundaries

External providers, AI APIs, model output, downloaded image files, exported paths, remote backup storage and local plugin/runtime components are untrusted inputs. The desktop UI is not a security boundary; validation must occur in application/domain or adapter layers.

## Principal threats and mitigations

- Credential leakage: OS credential store, redaction tests, no secret-bearing logs/manifests, explicit re-auth after restore.
- Path traversal or malicious filenames: internal IDs for storage paths, strict relative-path validation for export/restore, no model-controlled paths.
- Malformed/oversized media: byte limits, decoder validation, timeouts, resource limits, temporary-file cleanup.
- Prompt injection or fabricated AI content: treat output as untrusted, schema and semantic validation, protected-term checks, human review.
- Unintended external image disclosure: no image upload by default; require explicit per-batch approval before dispatch; show provider, image count, purpose and available cost estimate; cancel means no upload; do not reuse consent for future batches.
- Consent/audit leakage: record only minimal job/provider/scope/time/decision metadata; never include image bytes, API keys, cookies, tokens or full request bodies in logs.
- Crop proposal damage: crops are derived assets only; immutable original bytes/hash, human approval, coordinate/bounds validation and export opt-in.
- Remote backup corruption or partial publication: SHA-256 manifest, readback verification, immutable version prefix, update LATEST last.
- Accidental destructive operation: preview, explicit confirmation, quarantine, rollback metadata and audit trail.
- Provider policy violation: supported APIs/auth only, rate limiting, no CAPTCHA/paywall/access-control bypass.
- Dependency compromise: lockfile, dependency review, minimal feature sets, CI audit policy defined by maintainers.

## Privacy defaults

Do not upload images to AI by default. Each external vision-analysis batch requires a fresh explicit approval scoped to that batch and purpose. Do not treat one approval as global or future consent. Explain which image content and metadata are sent to each external service and provide cancellation before dispatch. Do not include API keys, cookies, access tokens or browser-session data in database snapshots or backups. Explain the local data location and provide deletion/export controls. Provider retention and training policies must be presented only when verified; do not make unsupported privacy guarantees.

## Security test cases

Secret redaction, malicious filename, `../` path, absolute restore path, oversized image, malformed image, AI output containing path/control payload, corrupted manifest, unexpected remote object, cancellation during destructive workflow, external vision upload blocked without batch approval, new batch requires new approval, cancellation causes no dispatch, and consent audit contains no image payload or secret.
