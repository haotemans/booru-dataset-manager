# Ingestion pipeline

Minimum original dimensions: width >= 800 AND height >= 800. Prefer original metadata dimensions; thumbnail dimensions are not a substitute. Unknown dimensions are skipped as resolution_unknown by default unless the user explicitly overrides. Verify actual dimensions after download.

Import flow: DISCOVERED → VALIDATING → HASHING → EXACT_DEDUP_CHECK → COMMITTED. Commit the valid media and provenance independently of DINOv3 availability. After commit, enqueue a separate analysis work unit: PENDING_EMBEDDING → DINO_EMBEDDING → SIMILARITY_SEARCH → MATCHED_IDENTITY | NEW_IDENTITY | NEEDS_REVIEW. If the model/runtime is unavailable, keep analysis pending with an actionable reason; do not roll back or block the imported media.

Download to temporary storage, validate actual image/dimensions, compute MD5 and check database uniqueness, compute SHA-256 integrity hash, move to managed library, commit metadata transaction, and reconcile interrupted operations at startup. No partial final file or dangling database row should survive recovery.

Planned providers: Danbooru, Gelbooru, Pixiv, X/Twitter, E-Hentai. Browser login does not automatically grant app access to browser cookies/session. Use supported site-specific authentication only; do not bypass CAPTCHA, paywalls, access controls or provider policies. Adapters need rate limits, retry/backoff, cancellation and auditable errors.


## Decoupled visual analysis

DINOv3 embedding and similarity analysis is queued by default after successful media commit, but is not part of the ingestion transaction's success condition. Analysis queue state, attempts, model/preprocessing version, errors and results are persisted separately. Users can start batch analysis for existing library items and re-run analysis after selecting a new model or preprocessing version. New analysis must preserve prior results and human-confirmed group/representative decisions in history; it must not silently replace them. Show import progress and analysis progress separately.
