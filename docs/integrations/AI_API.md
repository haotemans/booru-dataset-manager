# AI API integration and security

## Provider profiles

Support multiple named OpenAI-compatible profiles with base URL, model, timeout, bounded retries and concurrency. Keep separate profile/feature switches for tag optimization and caption generation. Test-connection and model-list operations must return redacted diagnostics. Never commit API keys or write them to logs, crash reports, backup manifests or ordinary settings files; use an OS credential store where available and clearly explain any fallback limitations.

Artist and character canonical names are removed from the AI rewrite input and merged back locally. Protected terms and aliases are passed as constraints only when needed; unknown terms remain unchanged and flagged for review. Raw tags are immutable provenance. Caption generation must be factual and avoid unsupported visual claims.

## JSON request and response contract

All AI API requests must be serialized as JSON. All accepted operation results must be parsed and validated as JSON; free-form prose, Markdown fences, or text scraping are not valid success responses.

- Define a versioned schema per operation (visual assessment, crop proposal, tag optimization, caption generation). Include explicit operation name/version, stable item ID, required fields, types, enums, and numeric/string/array bounds.
- For batch requests, give every input item a stable unique ID and require each result to identify that same ID. Validate exact correspondence: unexpected, duplicate, missing, or mismatched IDs are item-level failures; never join responses to inputs solely by array order.
- Use provider-native structured output or JSON Schema mode where supported. Maintain provider capability flags. If unavailable, still request strict JSON and use the same local parser/validator; do not claim the provider enforces a schema when it does not.
- Parse with a real JSON parser, then validate schema and domain invariants. Reject malformed JSON, wrong types, out-of-range values, extra or missing required fields, invalid crop rectangles, and unsupported status values. Do not silently repair a response into a success.
- Use bounded retry with a clear retry reason for transient transport errors and eligible malformed responses. Do not retry indefinitely. If a batch response is partial or one item is invalid, preserve accepted valid item results and retry only eligible failed items where possible.
- Persist job and per-item states so interrupted batches can resume idempotently. A repeated result must not overwrite a human-reviewed choice or create duplicate accepted outputs.
- Record provider/model, schema version, request/job ID, attempt count, validation outcome and error category. Keep diagnostics redacted; cap raw response retention and never log image bytes, secrets or credential-bearing headers.
- JSON validity is not semantic correctness. Apply protected-term checks and domain validation; vision-based aesthetic and crop proposals remain advisory and require human review.

## External vision API upload consent

Image analysis is a separate capability from text-only AI operations. External image upload is disabled unless the user explicitly approves the current batch.

Before dispatching a vision request, the application must:
1. Prepare the exact candidate batch and show the configured provider/profile, image count, analysis purpose, and any available cost/usage estimate.
2. Explain that image content will be transmitted to that external provider. Identify provider privacy/retention information separately when available; do not invent or guarantee provider behavior.
3. Let the user inspect the batch and remove items, approve the batch, or cancel.
4. Bind approval to that batch/job and purpose. Do not persist it as global consent or reuse it for later batches.
5. Block dispatch until approval is recorded. Cancellation, missing UI confirmation, or unavailable consent state means no upload; report the job as awaiting approval or cancelled.
6. Record a minimal consent audit event (timestamp, provider/profile identifier, job/batch identifier, scope/count, decision) without storing image bytes, credentials, or unnecessary sensitive content.

Background jobs may prepare a batch, but must pause before sending it until the user approves. Do not treat an earlier approval as authorization for a different batch or different purpose. Local-only analysis can proceed without external upload consent when a supported local analyzer is explicitly selected.

Never send API keys, cookies, provider session tokens, browser-session data, or unrelated local files to the vision endpoint. Do not log request bodies containing image data or credential-bearing headers. Redact diagnostics and ensure cancellation prevents not-yet-dispatched requests where possible.

## Vision assessment and crop suggestions

A vision model may assess aesthetic completion, visual clutter and visible text, and may propose crop coordinates with a rationale. A crop is only a derived proposal. Preserve original bytes and SHA-256; show original and proposed crop side by side. Require human approval or adjustment before using a crop in export. Store provider/model/prompt version, crop parameters, response status and review outcome. Treat model output as untrusted and validate schema, coordinates, bounds, aspect ratio and output size.

## Output validation and safety

Treat model output as untrusted data. Validate schema, length, encoding, protected-term coverage, unauthorized additions, and output target. Do not allow model output to choose filesystem paths, SQL fragments, shell commands, or remote repository names. Use bounded request/response sizes and cancellation. A failed target must not overwrite its last valid version or undo another target's successful result.

Acceptance tests: JSON serialization, strict JSON parsing, schema validation, unknown/duplicate/missing batch item IDs, malformed and oversized output, bounded retries, partial batch success, resume/idempotency, key redaction, timeout, protected artist/character names, unknown special tags, no image upload by default, batch consent required on every external vision job, cancellation/no-dispatch behavior, consent audit redaction, target isolation, and reviewed-output preservation.
