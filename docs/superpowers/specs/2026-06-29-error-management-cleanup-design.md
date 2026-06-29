# Error Management Cleanup for tracel-app

## Problem

`JobRegister` returns `Box<dyn Error + Send + Sync>` from all its methods. This untyped error forces both CLI and server to guess what went wrong, leading to:

- Duplicated "unknown job" error messages in 3 places
- `CliError::ConfigError` exists but is never constructed (dead code)
- `CliError::JobError` conflates config errors and execution errors
- Server handler uses raw `(StatusCode, String)` tuples instead of typed errors
- `expect()` panic in `RunFn` downcast because there's no typed error for type mismatches
- Triple HashMap lookup per request (`has_job` + `validate` + `run`) because the handler can't trust untyped errors to distinguish "not found" from "bad config"

## Design

### New: `JobRegisterError`

A shared error enum in `job_register.rs` covering all job dispatch failure modes:

```rust
#[derive(Debug, thiserror::Error)]
pub enum JobRegisterError {
    #[error("unknown job '{name}'. Available: {}", available.join(", "))]
    UnknownJob {
        name: String,
        available: Vec<String>,
    },

    #[error("validation failed: {0}")]
    ValidationFailed(#[source] Box<dyn Error + Send + Sync>),

    #[error("execution failed: {0}")]
    ExecutionFailed(#[source] Box<dyn Error + Send + Sync>),
}
```

- `UnknownJob`: job name not found in the register. Single source for the error message.
- `ValidationFailed`: mapper could not parse the config (bad JSON, missing field, wrong type). Wraps the serde error.
- `ExecutionFailed`: `job.execute()` returned an error (training crashed, etc.).

### Updated: `JobRegister` methods

All public methods return `JobRegisterError` instead of `Box<dyn Error>`:

- `validate(&self, job_name, config) -> Result<Box<dyn Any + Send>, JobRegisterError>` — returns `UnknownJob` or `ValidationFailed`
- `run(&self, job_name, input) -> Result<(), JobRegisterError>` — returns `UnknownJob` or `ExecutionFailed`. The current `expect()` panic on downcast failure becomes `ExecutionFailed` with a descriptive message.
- `dispatch(&self, job_name, config) -> Result<(), JobRegisterError>` — calls validate then run, can return any variant.

### Updated: `CliError`

```rust
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("no job name given and no default registered")]
    MissingDefault,

    #[error(transparent)]
    JobRegister(#[from] JobRegisterError),
}
```

Removes `UnknownJob`, `ConfigError`, and `JobError` — all covered by `JobRegisterError`. Only `MissingDefault` remains as a CLI-exclusive concern.

### Updated: `ServerError`

```rust
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("server error: {0}")]
    IoError(#[from] std::io::Error),

    #[error(transparent)]
    JobRegister(#[from] JobRegisterError),
}
```

Adds `JobRegister` variant so the server handler can use typed errors.

### Updated: `run_job` handler

The handler matches on `JobRegisterError` variants instead of using raw tuples and a separate `has_job()` check:

```rust
async fn run_job(...) -> impl IntoResponse {
    let input = match register.validate(&job_name, &body) {
        Ok(input) => input,
        Err(e @ JobRegisterError::UnknownJob { .. }) => return (StatusCode::NOT_FOUND, e.to_string()),
        Err(e @ JobRegisterError::ValidationFailed(_)) => return (StatusCode::BAD_REQUEST, e.to_string()),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    // spawn_blocking with run()...
}
```

This eliminates:
- The separate `has_job()` check (redundant lookup)
- The inline `format!()` for unknown jobs (uses `Display` from `JobRegisterError`)
- Raw tuple construction for error responses

### Updated: `ValidateFn` and `RunFn` signatures

```rust
pub type ValidateFn = Box<dyn Fn(&str) -> Result<Box<dyn Any + Send>, JobRegisterError> + Send + Sync>;
pub type RunFn = Box<dyn Fn(Box<dyn Any + Send>) -> Result<(), JobRegisterError> + Send + Sync>;
```

The closures in `erase_job` map their inner errors into the appropriate `JobRegisterError` variant. The downcast `expect()` becomes:

```rust
let input = *input.downcast::<I>().map_err(|_| {
    JobRegisterError::ExecutionFailed("internal type mismatch in job dispatch".into())
})?;
```

### CLI test impact

Existing tests match on `CliError::UnknownJob`, `CliError::JobError`, etc. These change to match on `CliError::JobRegister(JobRegisterError::UnknownJob { .. })` and `CliError::JobRegister(JobRegisterError::ValidationFailed(_))` / `CliError::JobRegister(JobRegisterError::ExecutionFailed(_))`.

## Files changed

| File | Change |
|------|--------|
| `job_register.rs` | Add `JobRegisterError` enum. Update `validate`, `run`, `dispatch` return types. Replace `expect()` with error. |
| `job.rs` | Update `ValidateFn` and `RunFn` to return `JobRegisterError`. |
| `cli/error.rs` | Remove `UnknownJob`, `ConfigError`, `JobError`. Add `JobRegister(#[from] JobRegisterError)`. |
| `cli/mod.rs` | Update `dispatch()` to use `?` with the new `From` impl. Remove manual error wrapping. |
| `cli/mod.rs` (tests) | Update `matches!` patterns to use `CliError::JobRegister(JobRegisterError::...)`. |
| `server/error.rs` | Add `JobRegister(#[from] JobRegisterError)`. |
| `server/mod.rs` | Replace `has_job()` + raw tuples with `match` on `JobRegisterError` variants. |
