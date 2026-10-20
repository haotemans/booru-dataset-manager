# Module boundaries and contracts

## Dependency rules

- `apps/desktop`: presentation, navigation, user input, progress display. It must not issue SQL or call provider HTTP APIs directly.
- `crates/application`: use cases, orchestration, cancellation, retries, job snapshots and transaction boundaries. Depends on domain and ports only.
- `crates/domain`: identifiers, value objects, validation rules and state transitions. No UI, database, HTTP, model runtime or filesystem dependency.
- `crates/ports`: traits for repositories, blob storage, source providers, embedding, AI completion, export and backup transport.
- `crates/infrastructure`: SQLite, filesystem, HTTP, image decoding, model inference and remote storage adapters.
- `crates/config`: typed non-secret settings. Secret values belong in the OS credential store or an explicitly supported secret provider, never plain committed config.

## Port contracts

All I/O ports should expose typed errors and cancellation where operations can be long-running. A repository method must document transaction scope and idempotency. A filesystem method must document whether it creates temporary files and who cleans them up. A remote upload method must distinguish uploaded, remotely verified, retryable failure, permanent failure and unknown outcome. Unknown outcome must be reconciled by querying remote state before retrying a non-idempotent operation.

## Job conventions

Every long-running job has a stable ID, immutable input snapshot, state, timestamps, progress counters, cancellation request, structured error code and retry policy. State changes are persisted before side effects when possible. Retries must be idempotent. Never report 100% or success before required verification completes.

## Errors

Use machine-readable error codes plus safe user-facing messages. Do not include API keys, authorization headers, cookies, signed URLs or raw secrets in logs. Preserve actionable context such as provider, media ID, stage and retryability. Avoid swallowing errors with default values.
