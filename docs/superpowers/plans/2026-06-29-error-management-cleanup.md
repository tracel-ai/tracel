# Error Management Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace untyped `Box<dyn Error>` returns in `JobRegister` with a typed `JobRegisterError` enum, and update CLI/server error types to wrap it.

**Architecture:** `JobRegisterError` becomes the single source of truth for job dispatch errors. `CliError` and `ServerError` each wrap it via `#[from]`, keeping only their entry-point-specific variants. The server handler matches on `JobRegisterError` variants for HTTP status codes instead of using raw tuples.

**Tech Stack:** Rust, thiserror, axum

## Global Constraints

- All existing CLI tests must pass after changes
- `job_register` module stays private (`mod`, not `pub mod`) — re-export `JobRegisterError` from `lib.rs`
- No new dependencies

---

### Task 1: Add `JobRegisterError` and update `JobRegister` methods

**Files:**
- Modify: `crates/tracel-app/src/job_register.rs`
- Modify: `crates/tracel-app/src/job.rs`
- Modify: `crates/tracel-app/src/lib.rs`

**Interfaces:**
- Produces: `JobRegisterError` enum with variants `UnknownJob`, `ValidationFailed`, `ExecutionFailed`
- Produces: `JobRegister::validate() -> Result<Box<dyn Any + Send>, JobRegisterError>`
- Produces: `JobRegister::run() -> Result<(), JobRegisterError>`
- Produces: `JobRegister::dispatch() -> Result<(), JobRegisterError>`

- [ ] **Step 1: Write failing tests for typed errors in `job_register.rs`**

Add a test module at the bottom of `crates/tracel-app/src/job_register.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::Job;
    use crate::mapper::Mapper;
    use std::error::Error;

    struct FakeJob {
        name: &'static str,
        should_fail: bool,
    }

    impl Job<String, ()> for FakeJob {
        fn name(&self) -> &str {
            self.name
        }

        fn execute(&self, _input: String) -> Result<(), Box<dyn Error + Send + Sync>> {
            if self.should_fail {
                Err("job execution failed".into())
            } else {
                Ok(())
            }
        }
    }

    struct FakeMapper {
        should_fail: bool,
    }

    impl Mapper<String> for FakeMapper {
        fn map(&self, raw: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
            if self.should_fail {
                Err("mapper failed".into())
            } else {
                Ok(raw.to_string())
            }
        }
    }

    #[test]
    fn validate_unknown_job_returns_unknown_job_error() {
        let register = JobRegister::new()
            .register(FakeJob { name: "train", should_fail: false }, FakeMapper { should_fail: false });

        let result = register.validate("infer", "{}");

        assert!(matches!(result, Err(JobRegisterError::UnknownJob { .. })));
    }

    #[test]
    fn validate_bad_config_returns_validation_failed() {
        let register = JobRegister::new()
            .register(FakeJob { name: "train", should_fail: false }, FakeMapper { should_fail: true });

        let result = register.validate("train", "{}");

        assert!(matches!(result, Err(JobRegisterError::ValidationFailed(_))));
    }

    #[test]
    fn validate_ok_returns_input() {
        let register = JobRegister::new()
            .register(FakeJob { name: "train", should_fail: false }, FakeMapper { should_fail: false });

        let result = register.validate("train", "hello");

        assert!(result.is_ok());
    }

    #[test]
    fn run_unknown_job_returns_unknown_job_error() {
        let register = JobRegister::new()
            .register(FakeJob { name: "train", should_fail: false }, FakeMapper { should_fail: false });

        let input: Box<dyn Any + Send> = Box::new("test".to_string());
        let result = register.run("infer", input);

        assert!(matches!(result, Err(JobRegisterError::UnknownJob { .. })));
    }

    #[test]
    fn run_execution_failure_returns_execution_failed() {
        let register = JobRegister::new()
            .register(FakeJob { name: "train", should_fail: true }, FakeMapper { should_fail: false });

        let input = register.validate("train", "{}").unwrap();
        let result = register.run("train", input);

        assert!(matches!(result, Err(JobRegisterError::ExecutionFailed(_))));
    }

    #[test]
    fn dispatch_ok() {
        let register = JobRegister::new()
            .register(FakeJob { name: "train", should_fail: false }, FakeMapper { should_fail: false });

        let result = register.dispatch("train", "hello");

        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_unknown_job() {
        let register = JobRegister::new()
            .register(FakeJob { name: "train", should_fail: false }, FakeMapper { should_fail: false });

        let result = register.dispatch("infer", "{}");

        assert!(matches!(result, Err(JobRegisterError::UnknownJob { .. })));
    }

    #[test]
    fn dispatch_validation_failed() {
        let register = JobRegister::new()
            .register(FakeJob { name: "train", should_fail: false }, FakeMapper { should_fail: true });

        let result = register.dispatch("train", "{}");

        assert!(matches!(result, Err(JobRegisterError::ValidationFailed(_))));
    }

    #[test]
    fn dispatch_execution_failed() {
        let register = JobRegister::new()
            .register(FakeJob { name: "train", should_fail: true }, FakeMapper { should_fail: false });

        let result = register.dispatch("train", "{}");

        assert!(matches!(result, Err(JobRegisterError::ExecutionFailed(_))));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p tracel-app -- job_register::tests -v`
Expected: compilation errors — `JobRegisterError` does not exist yet.

- [ ] **Step 3: Add `JobRegisterError` enum and update `ValidateFn`/`RunFn`**

In `crates/tracel-app/src/job_register.rs`, add at the top (after imports):

```rust
use std::error::Error;

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

In `crates/tracel-app/src/job.rs`, update the type aliases:

```rust
use std::{any::Any, error::Error};
use tracel_experiment::ExperimentJob;

use crate::job_register::JobRegisterError;

pub type ValidateFn =
    Box<dyn Fn(&str) -> Result<Box<dyn Any + Send>, JobRegisterError> + Send + Sync>;
pub type RunFn =
    Box<dyn Fn(Box<dyn Any + Send>) -> Result<(), JobRegisterError> + Send + Sync>;
```

- [ ] **Step 4: Update `erase_job` closures to produce `JobRegisterError`**

In `crates/tracel-app/src/job_register.rs`, update `erase_job`:

```rust
fn erase_job<J, I, O, M>(job: J, mapper: M) -> JobEntry
where
    J: Job<I, O> + Send + Sync + 'static,
    M: Mapper<I> + Send + Sync + 'static,
    I: Send + 'static,
    O: 'static,
{
    let validate: ValidateFn = Box::new(move |config_str: &str| {
        let input = mapper.map(config_str).map_err(JobRegisterError::ValidationFailed)?;
        Ok(Box::new(input) as Box<dyn Any + Send>)
    });

    let run: RunFn = Box::new(move |input: Box<dyn Any + Send>| {
        let input = *input
            .downcast::<I>()
            .map_err(|_| JobRegisterError::ExecutionFailed(
                "internal type mismatch in job dispatch".into(),
            ))?;
        job.execute(input).map(|_| ()).map_err(JobRegisterError::ExecutionFailed)
    });

    JobEntry { validate, run }
}
```

- [ ] **Step 5: Update `validate`, `run`, and `dispatch` return types**

In `crates/tracel-app/src/job_register.rs`, update the three public methods:

```rust
pub fn validate(
    &self,
    job_name: &str,
    config: &str,
) -> Result<Box<dyn Any + Send>, JobRegisterError> {
    let entry = self.jobs.get(job_name).ok_or_else(|| {
        JobRegisterError::UnknownJob {
            name: job_name.to_string(),
            available: self.job_names(),
        }
    })?;
    (entry.validate)(config)
}

pub fn run(
    &self,
    job_name: &str,
    input: Box<dyn Any + Send>,
) -> Result<(), JobRegisterError> {
    let entry = self.jobs.get(job_name).ok_or_else(|| {
        JobRegisterError::UnknownJob {
            name: job_name.to_string(),
            available: self.job_names(),
        }
    })?;
    (entry.run)(input)
}

pub fn dispatch(
    &self,
    job_name: &str,
    config: &str,
) -> Result<(), JobRegisterError> {
    let input = self.validate(job_name, config)?;
    self.run(job_name, input)
}
```

- [ ] **Step 6: Re-export `JobRegisterError` from `lib.rs`**

In `crates/tracel-app/src/lib.rs`, add a re-export so CLI and server can use it:

```rust
pub use job_register::JobRegisterError;
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p tracel-app -- job_register::tests -v`
Expected: all 9 new tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/tracel-app/src/job_register.rs crates/tracel-app/src/job.rs crates/tracel-app/src/lib.rs
git commit -m "feat: add JobRegisterError with typed variants for validate/run/dispatch"
```

---

### Task 2: Update `CliError` and CLI dispatch

**Files:**
- Modify: `crates/tracel-app/src/cli/error.rs`
- Modify: `crates/tracel-app/src/cli/mod.rs` (dispatch + tests)

**Interfaces:**
- Consumes: `JobRegisterError` from Task 1
- Consumes: `JobRegister::dispatch() -> Result<(), JobRegisterError>`
- Produces: `CliError` with variants `MissingDefault` and `JobRegister(JobRegisterError)`

- [ ] **Step 1: Update `CliError` enum**

Replace `crates/tracel-app/src/cli/error.rs` with:

```rust
use crate::job_register::JobRegisterError;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("no job name given and no default registered")]
    MissingDefault,

    #[error(transparent)]
    JobRegister(#[from] JobRegisterError),
}
```

- [ ] **Step 2: Update `Cli::dispatch` to use the new error types**

In `crates/tracel-app/src/cli/mod.rs`, replace the `dispatch` method:

```rust
fn dispatch(self, job: Option<String>, config: Option<String>) -> Result<(), CliError> {
    match job {
        Some(job_name) => {
            let config_str = config.unwrap_or_default();
            self.register.dispatch(&job_name, &config_str)?;
            Ok(())
        }
        None => {
            let d = self.default.ok_or(CliError::MissingDefault)?;
            (d.runner)().map_err(|e| {
                CliError::JobRegister(JobRegisterError::ExecutionFailed(e))
            })
        }
    }
}
```

Also remove the unused `use std::error::Error;` import if it becomes unused after this change.

- [ ] **Step 3: Update test assertions to match new error variants**

In the test module of `crates/tracel-app/src/cli/mod.rs`, add the import and update assertions:

Add to test imports:
```rust
use crate::job_register::JobRegisterError;
```

Update these tests:

`dispatch_named_job_unknown`:
```rust
assert!(matches!(
    result,
    Err(CliError::JobRegister(JobRegisterError::UnknownJob { .. }))
));
```

`mapper_error_is_wrapped_in_job_error` — rename to `mapper_error_is_validation_failed`:
```rust
#[test]
fn mapper_error_is_validation_failed() {
    let cli = Cli::new().register(FakeJob::new("train"), FakeMapper::failing());

    let result = cli.dispatch(Some("train".into()), Some("{}".into()));

    assert!(matches!(
        result,
        Err(CliError::JobRegister(JobRegisterError::ValidationFailed(_)))
    ));
}
```

`job_error_is_wrapped_in_job_error` — rename to `job_error_is_execution_failed`:
```rust
#[test]
fn job_error_is_execution_failed() {
    let cli = Cli::new().register(FakeJob::failing("train"), FakeMapper::new());

    let result = cli.dispatch(Some("train".into()), Some("{}".into()));

    assert!(matches!(
        result,
        Err(CliError::JobRegister(JobRegisterError::ExecutionFailed(_)))
    ));
}
```

`dispatch_default_job_fails`:
```rust
assert!(matches!(
    result,
    Err(CliError::JobRegister(JobRegisterError::ExecutionFailed(_)))
));
```

- [ ] **Step 4: Run all CLI tests**

Run: `cargo test -p tracel-app -- cli::tests -v`
Expected: all 10 tests pass (including renamed ones).

- [ ] **Step 5: Commit**

```bash
git add crates/tracel-app/src/cli/error.rs crates/tracel-app/src/cli/mod.rs
git commit -m "refactor: update CliError to wrap JobRegisterError"
```

---

### Task 3: Update `ServerError` and server handler

**Files:**
- Modify: `crates/tracel-app/src/server/error.rs`
- Modify: `crates/tracel-app/src/server/mod.rs`

**Interfaces:**
- Consumes: `JobRegisterError` from Task 1
- Consumes: `JobRegister::validate() -> Result<Box<dyn Any + Send>, JobRegisterError>`
- Consumes: `JobRegister::run() -> Result<(), JobRegisterError>`

- [ ] **Step 1: Update `ServerError` enum**

Replace `crates/tracel-app/src/server/error.rs` with:

```rust
use crate::job_register::JobRegisterError;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("server error: {0}")]
    IoError(#[from] std::io::Error),

    #[error(transparent)]
    JobRegister(#[from] JobRegisterError),
}
```

- [ ] **Step 2: Update `run_job` handler to match on typed errors**

In `crates/tracel-app/src/server/mod.rs`, replace the `run_job` function:

```rust
async fn run_job(
    State(register): State<Arc<JobRegister>>,
    Path(job_name): Path<String>,
    body: String,
) -> impl IntoResponse {
    let input = match register.validate(&job_name, &body) {
        Ok(input) => input,
        Err(e @ JobRegisterError::UnknownJob { .. }) => {
            return (StatusCode::NOT_FOUND, e.to_string());
        }
        Err(e @ JobRegisterError::ValidationFailed(_)) => {
            return (StatusCode::BAD_REQUEST, e.to_string());
        }
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
        }
    };

    let response_name = job_name.clone();

    tokio::task::spawn_blocking(move || {
        if let Err(e) = register.run(&job_name, input) {
            eprintln!("Job '{job_name}' failed: {e}");
        }
    });

    (
        StatusCode::OK,
        format!("Job '{response_name}' has started running"),
    )
}
```

Also add the import at the top of the file:

```rust
use crate::job_register::JobRegisterError;
```

And remove the now-unused `has_job` related code — the `has_job` method on `JobRegister` can stay (it may be useful elsewhere), but the handler no longer calls it.

- [ ] **Step 3: Run full test suite and compile check**

Run: `cargo test -p tracel-app -v && cargo check -p tracel-app --features server`
Expected: all tests pass, no compilation errors.

- [ ] **Step 4: Commit**

```bash
git add crates/tracel-app/src/server/error.rs crates/tracel-app/src/server/mod.rs
git commit -m "refactor: update ServerError and handler to use JobRegisterError"
```

---

### Task 4: Final verification

**Files:** None (verification only)

- [ ] **Step 1: Run the full workspace check**

Run: `cargo check --workspace`
Expected: no errors across all crates (including `tracel` re-export crate and `mnist` examples).

- [ ] **Step 2: Run all tests**

Run: `cargo test -p tracel-app -v`
Expected: all tests pass — both the new `job_register::tests` and the updated `cli::tests`.

- [ ] **Step 3: Verify no dead code warnings**

Run: `cargo check -p tracel-app 2>&1 | grep -i "unused\|dead"`
Expected: no warnings about unused error variants or dead code.
