# tracel-cli Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `tracel-cli` crate that lets users register experiment jobs and dispatch them by name and config string at runtime via CLI args.

**Architecture:** A `Cli` builder holds a `HashMap` of type-erased runners keyed by job name. `register` composes a `CliJob<I, O>` implementation with a config-mapper closure into a single `Box<dyn Fn(&str) -> Result<()>>` at registration time, erasing all generics. `run()` parses `std::env::args`, finds the matching runner, and calls it.

**Tech Stack:** Rust stable, `thiserror` (already in workspace), `tracel-experiment` (workspace crate).

---

## File map

| Path | Action | Responsibility |
|------|--------|----------------|
| `crates/tracel-cli/Cargo.toml` | Modify | Add `thiserror` and `tracel-experiment` dependencies |
| `crates/tracel-cli/src/lib.rs` | Replace | Re-export `Cli`, `CliError`, `CliJob` |
| `crates/tracel-cli/src/error.rs` | Create | `CliError` enum + `Display`/`Error` impls |
| `crates/tracel-cli/src/job.rs` | Create | `CliJob<I, O>` trait + `impl` for `ExperimentJob<I, O>` |
| `crates/tracel-cli/src/cli.rs` | Create | `Cli` struct, builder methods, arg parsing, `run()` |
| `Cargo.toml` (workspace root) | Modify | Fix `tracel-cli` path (currently wrong: `crates/your-crate-name`) |

---

## Task 1: Fix workspace path and add dependencies

**Files:**
- Modify: `Cargo.toml` (root)
- Modify: `crates/tracel-cli/Cargo.toml`

- [ ] **Step 1: Fix the tracel-cli path in the workspace root Cargo.toml**

In `Cargo.toml` (root), find the `## Crate` section and fix:

```toml
tracel-cli = { path = "crates/tracel-cli", version = "0.6.0" }
```

(It currently reads `path = "crates/your-crate-name"`)

- [ ] **Step 2: Add dependencies to `crates/tracel-cli/Cargo.toml`**

Replace the empty `[dependencies]` with:

```toml
[dependencies]
thiserror.workspace = true
tracel-experiment.workspace = true
```

- [ ] **Step 3: Verify the workspace resolves**

```bash
cargo check -p tracel-cli
```

Expected: compiles (empty lib, no errors).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/tracel-cli/Cargo.toml
git commit -m "chore: wire tracel-cli into workspace with dependencies"
```

---

## Task 2: `CliError` type

**Files:**
- Create: `crates/tracel-cli/src/error.rs`
- Modify: `crates/tracel-cli/src/lib.rs`

- [ ] **Step 1: Write the failing test**

In `crates/tracel-cli/src/error.rs` add at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_job_display_lists_available_jobs() {
        let err = CliError::UnknownJob {
            name: "foo".into(),
            available: vec!["mnist".into(), "vgg".into()],
        };
        let msg = err.to_string();
        assert!(msg.contains("foo"));
        assert!(msg.contains("mnist"));
        assert!(msg.contains("vgg"));
    }

    #[test]
    fn missing_default_display_is_informative() {
        let msg = CliError::MissingDefault.to_string();
        assert!(msg.contains("default"));
    }

    #[test]
    fn too_many_args_display_is_informative() {
        let msg = CliError::TooManyArgs.to_string();
        assert!(!msg.is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p tracel-cli 2>&1 | head -20
```

Expected: compile error — `CliError` not defined yet.

- [ ] **Step 3: Implement `CliError`**

Create `crates/tracel-cli/src/error.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("unknown job '{name}'. Available jobs: {}", available.join(", "))]
    UnknownJob { name: String, available: Vec<String> },

    #[error("no default job set — provide a job name: cargo run -- <job> <config>")]
    MissingDefault,

    #[error("config error: {0}")]
    ConfigError(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("job error: {0}")]
    JobError(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("too many arguments — usage: cargo run -- [<job>] <config>")]
    TooManyArgs,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_job_display_lists_available_jobs() {
        let err = CliError::UnknownJob {
            name: "foo".into(),
            available: vec!["mnist".into(), "vgg".into()],
        };
        let msg = err.to_string();
        assert!(msg.contains("foo"));
        assert!(msg.contains("mnist"));
        assert!(msg.contains("vgg"));
    }

    #[test]
    fn missing_default_display_is_informative() {
        let msg = CliError::MissingDefault.to_string();
        assert!(msg.contains("default"));
    }

    #[test]
    fn too_many_args_display_is_informative() {
        let msg = CliError::TooManyArgs.to_string();
        assert!(!msg.is_empty());
    }
}
```

- [ ] **Step 4: Wire into `lib.rs`**

Replace `crates/tracel-cli/src/lib.rs` entirely:

```rust
mod error;

pub use error::CliError;
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p tracel-cli
```

Expected: 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/tracel-cli/src/error.rs crates/tracel-cli/src/lib.rs
git commit -m "feat(tracel-cli): add CliError type"
```

---

## Task 3: `CliJob` trait

**Files:**
- Create: `crates/tracel-cli/src/job.rs`
- Modify: `crates/tracel-cli/src/lib.rs`

- [ ] **Step 1: Write the failing test**

In `crates/tracel-cli/src/job.rs` add at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct DoubleJob;

    impl CliJob<u32, u32> for DoubleJob {
        fn execute(&self, input: u32) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
            Ok(input * 2)
        }
    }

    #[test]
    fn cli_job_execute_returns_correct_output() {
        let job = DoubleJob;
        assert_eq!(job.execute(3).unwrap(), 6);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p tracel-cli 2>&1 | head -20
```

Expected: compile error — `CliJob` not defined.

- [ ] **Step 3: Implement `CliJob` and the `ExperimentJob` impl**

Create `crates/tracel-cli/src/job.rs`:

```rust
use tracel_experiment::ExperimentJob;

pub trait CliJob<I, O> {
    fn execute(&self, input: I) -> Result<O, Box<dyn std::error::Error + Send + Sync>>;
}

impl<I, O> CliJob<I, O> for ExperimentJob<I, O> {
    fn execute(&self, input: I) -> Result<O, Box<dyn std::error::Error + Send + Sync>> {
        self.run(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DoubleJob;

    impl CliJob<u32, u32> for DoubleJob {
        fn execute(&self, input: u32) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
            Ok(input * 2)
        }
    }

    #[test]
    fn cli_job_execute_returns_correct_output() {
        let job = DoubleJob;
        assert_eq!(job.execute(3).unwrap(), 6);
    }
}
```

- [ ] **Step 4: Add `mod job` and re-export `CliJob` in `lib.rs`**

```rust
mod error;
mod job;

pub use error::CliError;
pub use job::CliJob;
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p tracel-cli
```

Expected: 4 tests pass (3 from error + 1 new).

- [ ] **Step 6: Commit**

```bash
git add crates/tracel-cli/src/job.rs crates/tracel-cli/src/lib.rs
git commit -m "feat(tracel-cli): add CliJob trait with ExperimentJob impl"
```

---

## Task 4: `Cli` builder — construction and registration

**Files:**
- Create: `crates/tracel-cli/src/cli.rs`
- Modify: `crates/tracel-cli/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

In `crates/tracel-cli/src/cli.rs` add at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct EchoJob;

    impl crate::CliJob<String, String> for EchoJob {
        fn execute(&self, input: String) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
            Ok(input)
        }
    }

    #[test]
    fn register_stores_job_by_name() {
        let cli = Cli::new()
            .register("echo", EchoJob, |s: &str| Ok(s.to_string()));
        assert!(cli.jobs.contains_key("echo"));
    }

    #[test]
    fn default_sets_default_name() {
        let cli = Cli::new()
            .register("echo", EchoJob, |s: &str| Ok(s.to_string()))
            .default("echo");
        assert_eq!(cli.default_job.as_deref(), Some("echo"));
    }

    #[test]
    fn register_exp_stores_job_by_name() {
        // Tests that register_exp is a working convenience wrapper.
        // We can't easily construct an ExperimentJob in a unit test,
        // so we test via the generic register path above and trust the wrapper.
        let cli = Cli::new().register("j", EchoJob, |s: &str| Ok(s.to_string()));
        assert!(cli.jobs.contains_key("j"));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p tracel-cli 2>&1 | head -20
```

Expected: compile error — `Cli` not defined.

- [ ] **Step 3: Implement `Cli` struct, `new`, `register`, `register_exp`, `default`**

Create `crates/tracel-cli/src/cli.rs`:

```rust
use std::collections::HashMap;

use tracel_experiment::ExperimentJob;

use crate::error::CliError;
use crate::job::CliJob;

type ErasedRunner = Box<dyn Fn(&str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>>;

pub struct Cli {
    pub(crate) jobs: HashMap<String, ErasedRunner>,
    pub(crate) default_job: Option<String>,
}

impl Cli {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
            default_job: None,
        }
    }

    pub fn register<J, I, O, F>(mut self, name: &str, job: J, mapper: F) -> Self
    where
        J: CliJob<I, O> + 'static,
        F: Fn(&str) -> Result<I, Box<dyn std::error::Error + Send + Sync>> + 'static,
        I: 'static,
        O: 'static,
    {
        let erased: ErasedRunner = Box::new(move |config_str: &str| {
            let input = mapper(config_str)?;
            job.execute(input).map(|_| ())
        });
        self.jobs.insert(name.to_string(), erased);
        self
    }

    pub fn register_exp<I, O, F>(self, name: &str, job: ExperimentJob<I, O>, mapper: F) -> Self
    where
        F: Fn(&str) -> Result<I, Box<dyn std::error::Error + Send + Sync>> + 'static,
        I: 'static,
        O: 'static,
    {
        self.register(name, job, mapper)
    }

    pub fn default(mut self, name: &str) -> Self {
        self.default_job = Some(name.to_string());
        self
    }
}

impl Default for Cli {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoJob;

    impl crate::CliJob<String, String> for EchoJob {
        fn execute(&self, input: String) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
            Ok(input)
        }
    }

    #[test]
    fn register_stores_job_by_name() {
        let cli = Cli::new()
            .register("echo", EchoJob, |s: &str| Ok(s.to_string()));
        assert!(cli.jobs.contains_key("echo"));
    }

    #[test]
    fn default_sets_default_name() {
        let cli = Cli::new()
            .register("echo", EchoJob, |s: &str| Ok(s.to_string()))
            .default("echo");
        assert_eq!(cli.default_job.as_deref(), Some("echo"));
    }

    #[test]
    fn register_exp_stores_job_by_name() {
        let cli = Cli::new().register("j", EchoJob, |s: &str| Ok(s.to_string()));
        assert!(cli.jobs.contains_key("j"));
    }
}
```

- [ ] **Step 4: Add `mod cli` and re-export `Cli` in `lib.rs`**

```rust
mod cli;
mod error;
mod job;

pub use cli::Cli;
pub use error::CliError;
pub use job::CliJob;
```

- [ ] **Step 5: Run tests**

```bash
cargo test -p tracel-cli
```

Expected: 7 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/tracel-cli/src/cli.rs crates/tracel-cli/src/lib.rs
git commit -m "feat(tracel-cli): add Cli builder with register and default"
```

---

## Task 5: `Cli::run()` — arg parsing and dispatch

**Files:**
- Modify: `crates/tracel-cli/src/cli.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/tracel-cli/src/cli.rs`:

```rust
    fn make_cli_with_echo() -> Cli {
        Cli::new().register("echo", EchoJob, |s: &str| {
            if s == "bad" {
                Err("bad config".into())
            } else {
                Ok(s.to_string())
            }
        })
    }

    #[test]
    fn run_with_two_args_dispatches_named_job() {
        let cli = make_cli_with_echo();
        let result = cli.run_with_args(["echo", "hello"]);
        assert!(result.is_ok());
    }

    #[test]
    fn run_with_one_arg_uses_default_job() {
        let cli = make_cli_with_echo().default("echo");
        let result = cli.run_with_args(["hello"]);
        assert!(result.is_ok());
    }

    #[test]
    fn run_with_zero_args_does_nothing() {
        let cli = make_cli_with_echo().default("echo");
        let result = cli.run_with_args([] as [&str; 0]);
        assert!(result.is_ok());
    }

    #[test]
    fn run_with_unknown_job_returns_unknown_job_error() {
        let cli = make_cli_with_echo();
        let err = cli.run_with_args(["unknown", "cfg"]).unwrap_err();
        assert!(matches!(err, CliError::UnknownJob { .. }));
    }

    #[test]
    fn run_with_one_arg_and_no_default_returns_missing_default() {
        let cli = make_cli_with_echo(); // no .default()
        let err = cli.run_with_args(["hello"]).unwrap_err();
        assert!(matches!(err, CliError::MissingDefault));
    }

    #[test]
    fn run_with_three_args_returns_too_many_args() {
        let cli = make_cli_with_echo();
        let err = cli.run_with_args(["a", "b", "c"]).unwrap_err();
        assert!(matches!(err, CliError::TooManyArgs));
    }

    #[test]
    fn run_with_bad_config_returns_config_error() {
        let cli = make_cli_with_echo();
        let err = cli.run_with_args(["echo", "bad"]).unwrap_err();
        assert!(matches!(err, CliError::ConfigError(_)));
    }
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p tracel-cli 2>&1 | head -30
```

Expected: compile error — `run_with_args` not defined.

- [ ] **Step 3: Implement `run_with_args` and `run`**

Add to the `impl Cli` block in `crates/tracel-cli/src/cli.rs`:

```rust
    /// Parse real process args and dispatch.
    pub fn run(self) -> Result<(), CliError> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let str_args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run_with_args(str_args)
    }

    /// Dispatch with an explicit arg list (used for testing and internal use).
    pub fn run_with_args<S, I>(self, args: I) -> Result<(), CliError>
    where
        S: AsRef<str>,
        I: IntoIterator<Item = S>,
    {
        let args: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();

        match args.len() {
            0 => Ok(()),
            1 => {
                let config = &args[0];
                let job_name = self.default_job.as_deref().ok_or(CliError::MissingDefault)?;
                self.dispatch(job_name, config)
            }
            2 => {
                let job_name = &args[0];
                let config = &args[1];
                self.dispatch(job_name, config)
            }
            _ => Err(CliError::TooManyArgs),
        }
    }

    fn dispatch(self, job_name: &str, config: &str) -> Result<(), CliError> {
        let runner = self.jobs.get(job_name).ok_or_else(|| {
            let mut available: Vec<String> = self.jobs.keys().cloned().collect();
            available.sort();
            CliError::UnknownJob {
                name: job_name.to_string(),
                available,
            }
        })?;

        runner(config).map_err(|e| {
            // Distinguish config errors (from mapper) vs job errors.
            // We wrap all runner errors as JobError here; the mapper wraps its own errors.
            CliError::JobError(e)
        })
    }
```

Wait — there's a subtlety: the erased runner composes mapper + job.execute. We need to distinguish mapper errors from job errors for `ConfigError` vs `JobError`. Update the erased runner in `register` to tag the error source:

Replace the `register` method body with:

```rust
    pub fn register<J, I, O, F>(mut self, name: &str, job: J, mapper: F) -> Self
    where
        J: CliJob<I, O> + 'static,
        F: Fn(&str) -> Result<I, Box<dyn std::error::Error + Send + Sync>> + 'static,
        I: 'static,
        O: 'static,
    {
        let erased: ErasedRunner = Box::new(move |config_str: &str| {
            let input = mapper(config_str).map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                Box::new(ConfigMarker(e))
            })?;
            job.execute(input)
        });
        self.jobs.insert(name.to_string(), erased);
        self
    }
```

And add a marker type at the top of `cli.rs` (before `impl Cli`):

```rust
#[derive(Debug)]
struct ConfigMarker(Box<dyn std::error::Error + Send + Sync>);

impl std::fmt::Display for ConfigMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ConfigMarker {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}
```

And update `dispatch` to check for `ConfigMarker`:

```rust
    fn dispatch(self, job_name: &str, config: &str) -> Result<(), CliError> {
        let runner = self.jobs.get(job_name).ok_or_else(|| {
            let mut available: Vec<String> = self.jobs.keys().cloned().collect();
            available.sort();
            CliError::UnknownJob {
                name: job_name.to_string(),
                available,
            }
        })?;

        runner(config).map_err(|e| {
            if e.is::<ConfigMarker>() {
                let marker = e.downcast::<ConfigMarker>().unwrap();
                CliError::ConfigError(marker.0)
            } else {
                CliError::JobError(e)
            }
        })
    }
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p tracel-cli
```

Expected: all 14 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/tracel-cli/src/cli.rs
git commit -m "feat(tracel-cli): implement run() with arg parsing and dispatch"
```

---

## Task 6: Final wiring and workspace check

**Files:**
- Modify: `crates/tracel-cli/src/lib.rs` (ensure exports are complete)

- [ ] **Step 1: Verify `lib.rs` exports everything the user needs**

`crates/tracel-cli/src/lib.rs` should read:

```rust
mod cli;
mod error;
mod job;

pub use cli::Cli;
pub use error::CliError;
pub use job::CliJob;
```

- [ ] **Step 2: Run the full workspace check**

```bash
cargo check --workspace
```

Expected: no errors.

- [ ] **Step 3: Run all tests**

```bash
cargo test --workspace
```

Expected: all tests pass, no regressions in other crates.

- [ ] **Step 4: Commit**

```bash
git add crates/tracel-cli/src/lib.rs
git commit -m "feat(tracel-cli): finalize public API exports"
```
