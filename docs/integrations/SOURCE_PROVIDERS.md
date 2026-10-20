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


## Preferred source and fallback

Use Danbooru as the default primary source. If a task cannot feasibly proceed through Danbooru, Gelbooru is the fallback. Do not harvest both sources in parallel by default. Record the selected provider and any failover reason in task provenance, and route both providers through the same normalized ingestion pipeline. Failover must respect authentication, rate limits and site policies. Automatic failover conditions must be defined explicitly before implementation; transient throttling or service errors must not be treated as permission to bypass a provider's rate limits.


### Danbooru-to-Gelbooru fallback conditions

For a Danbooru-first task, Gelbooru may be used when Danbooru cannot be used for the task, or when Danbooru produces no items that pass configured eligibility filters such as the original-dimension gate and actual-file validation. No matching results and all-results-excluded outcomes qualify. This policy does not automatically top up a partially met numeric target from Gelbooru. Handle transient network errors and HTTP 429 using the primary provider's bounded retry/cooldown policy before deciding whether to fail over; never switch providers to evade rate limits, authentication requirements, policy restrictions or access controls. Persist the failover reason and source provenance, and use the common deduplication pipeline across both providers.


## Source-provided relationships

When a provider exposes trustworthy post-to-post, parent/child, pool, difference-set or alternate-version relationships, the adapter may normalize them as relationship records with relation type, provider, remote IDs/URLs and retrieval time. Preserve the source relationship provenance and distinguish direct provider links from DINOv3 suggestions and human-confirmed links. Do not assume every provider exposes these fields, and do not invent relations when unavailable. A provider relationship creates an association candidate, not an exact-duplicate assertion and not permission to delete any file.


### Difference-set relationship handling

When a source API explicitly provides a reliable difference-set relationship, automatically create the corresponding association group and persist provider ID/URL, relationship type and retrieval provenance. For representative-only dataset selection, choose one member of each difference set; selection can be automatic using the configured quality ranking or manual by the user. Keep every original variant in the managed library, and preserve an all-variants export option. Do not infer a source-confirmed difference relationship from DINOv3 similarity alone. If the adapter cannot reliably obtain source relationship metadata, it must not fabricate the relationship.
