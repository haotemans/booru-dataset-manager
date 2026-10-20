# AI API integration and security

Support multiple named OpenAI-compatible profiles with base URL, model, timeout, bounded retries and concurrency. Keep separate profile/feature switches for tag optimization and caption generation. Test-connection and model-list operations must return redacted diagnostics. Never commit API keys or write them to logs, crash reports, backup manifests or ordinary settings files; use an OS credential store where available and clearly explain any fallback limitations.

Artist and character canonical names are removed from the AI rewrite input and merged back locally. Protected terms and aliases are passed as constraints only when needed; unknown terms remain unchanged and flagged for review. Raw tags are immutable provenance. Caption generation must be factual and avoid unsupported visual claims. AI image upload is disabled unless the user explicitly opts in per profile/workflow. Credentials and provider session tokens must never be sent to an AI endpoint.

Treat model output as untrusted data. Validate schema, length, encoding, protected-term coverage, unauthorized additions, and output target. Do not allow model output to choose filesystem paths, SQL fragments, shell commands, or remote repository names. Use bounded request/response sizes and cancellation. A failed target must not overwrite its last valid version or undo another target's successful result.

Acceptance tests: key redaction, timeout, retry limit, malformed output, protected artist/character names, unknown special tags, no image upload by default, target isolation, and reviewed-output preservation.
