# Source provider adapters

## Common contract

Each provider adapter returns normalized source records: provider ID, canonical page URL, remote media ID, original-file URL when legitimately available, original dimensions when known, raw tags as supplied, artist/character/work metadata, publication time if supplied, and provider-specific provenance. Preserve the raw provider payload only when useful and lawful; do not store secrets in provenance.

Discovery, metadata validation and download are separate operations. Rate limits, retry-after responses, pagination, cancellation, timeouts, deduplication and partial failures must be handled independently. Retry only transient errors; use bounded exponential backoff with jitter and respect provider rate limits. Persist cursors/checkpoints so restart does not require rescanning everything.

## Authentication boundaries

Danbooru and Gelbooru adapters must use their supported API/authentication mechanisms. Pixiv, X/Twitter and E-Hentai support must be limited to currently supported, user-authorized mechanisms and provider policies. Opening a login page in the system browser does not grant the desktop app access to that browser's session. Do not extract browser cookies, defeat MFA/CAPTCHA, bypass paywalls, or evade access restrictions. If no supported authentication mechanism is available, expose the provider as unavailable with a precise explanation rather than pretending it works.

## Original file and dimensions

Prefer provider-declared original dimensions and original media URLs. Never apply the 800×800 gate to thumbnail dimensions when original dimensions are available. If dimensions are unknown, skip by default with `resolution_unknown`; explicit override must be recorded. After download, decode the actual file and verify dimensions, format, size and integrity before importing it.

## Acceptance criteria per provider

- API pagination and cursor recovery tests
- Rate-limit and retry-after tests
- Authentication failure and expired-session handling
- Original-versus-thumbnail selection tests
- Cancellation and timeout tests
- No credential leakage in logs
- A documented live smoke test against a user-authorized account before marking provider verified
