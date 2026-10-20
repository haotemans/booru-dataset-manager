# Security threat model

## Assets

Original media, raw source metadata, user-authored captions/tags, provider credentials, AI API keys, private backup repository, database and backup manifests.

## Trust boundaries

External providers, AI APIs, model output, downloaded image files, exported paths, remote backup storage and local plugin/runtime components are untrusted inputs. The desktop UI is not a security boundary; validation must occur in application/domain or adapter layers.

## Principal threats and mitigations

- Credential leakage: OS credential store, redaction tests, no secret-bearing logs/manifests, explicit re-auth after restore.
- Path traversal or malicious filenames: internal IDs for storage paths, strict relative-path validation for export/restore, no model-controlled paths.
- Malformed/oversized media: byte limits, decoder validation, timeouts, resource limits, temporary-file cleanup.
- Prompt injection or fabricated AI content: treat output as untrusted, schema and semantic validation, protected-term checks, human review.
- Remote backup corruption or partial publication: SHA-256 manifest, readback verification, immutable version prefix, update LATEST last.
- Accidental destructive operation: preview, explicit confirmation, quarantine, rollback metadata and audit trail.
- Provider policy violation: supported APIs/auth only, rate limiting, no CAPTCHA/paywall/access-control bypass.
- Dependency compromise: lockfile, dependency review, minimal feature sets, CI audit policy defined by maintainers.

## Privacy defaults

Do not upload images to AI by default. Do not include API keys, cookies, access tokens or browser-session data in database snapshots or backups. Explain which metadata is sent to each external service. Provide a clear local data location and deletion/export controls.

## Security test cases

Secret redaction, malicious filename, `../` path, absolute restore path, oversized image, malformed image, AI output containing path/control payload, corrupted manifest, unexpected remote object, and cancellation during destructive workflow.
