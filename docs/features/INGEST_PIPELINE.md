# Ingestion pipeline

Minimum original dimensions: width >= 800 AND height >= 800. Prefer original metadata dimensions; thumbnail dimensions are not a substitute. Unknown dimensions are skipped as resolution_unknown by default unless the user explicitly overrides. Verify actual dimensions after download.

State flow: DISCOVERED → VALIDATING → HASHING → EXACT_DEDUP_CHECK → PENDING_EMBEDDING → DINO_EMBEDDING → SIMILARITY_SEARCH → MATCHED_IDENTITY | NEW_IDENTITY | NEEDS_REVIEW → COMMITTED. A missing DINO model leaves the validated image pending embedding.

Download to temporary storage, validate actual image/dimensions, compute MD5 and check database uniqueness, compute SHA-256 integrity hash, move to managed library, commit metadata transaction, and reconcile interrupted operations at startup. No partial final file or dangling database row should survive recovery.

Planned providers: Danbooru, Gelbooru, Pixiv, X/Twitter, E-Hentai. Browser login does not automatically grant app access to browser cookies/session. Use supported site-specific authentication only; do not bypass CAPTCHA, paywalls, access controls or provider policies. Adapters need rate limits, retry/backoff, cancellation and auditable errors.
