# Automatic Ctrl-C Cancellation for Experiment Jobs

## Context

Previously, SDK example/integration code defined its own `install_ctrlc(experiment: &ExperimentRun)` helper and called it from the closure passed to the job runner, wiring `ctrlc::set_handler` to `experiment.cancel_token().cancel()`. The coach asked to move this into the SDK itself so users don't need to know about `ctrlc` at all.

The active "base closure" that wraps every user job is `ExperimentJob::run()` in `crates/tracel-experiment/src/provider.rs`. It creates the `ExperimentRun`, runs the user's function inside `handle.in_scope(...)`, then calls `experiment.finish()` or `experiment.fail()`. (`tracel-runtime::Executor::run()` is the deprecated equivalent and is out of scope.)

## Design

- Add `CancelToken::cancel_on_ctrlc(&self)` in `crates/tracel-experiment/src/cancellation.rs`, alongside the existing `link`/`linked` builder-style methods. It clones `self`, registers a `ctrlc::set_handler` closure that calls `.cancel()` and prints `"Received Ctrl-C, sending cancellation request..."`, and ignores the `Result` from `set_handler` (best-effort `let _ = ...`).
  - Ignoring the error matters because `ctrlc::set_handler` only succeeds once per process; if `ExperimentJob::run()` is ever called more than once in the same process (multiple jobs, future tests), a second registration must not panic.
- In `ExperimentJob::run()` (`crates/tracel-experiment/src/provider.rs:91`), call `experiment.cancel_token().cancel_on_ctrlc()` immediately after `let experiment = self.provider.create_experiment(...)?;` and before `handle.in_scope(...)`.
- Add `ctrlc = "3.5"` to `[workspace.dependencies]` in the root `Cargo.toml`, and `ctrlc.workspace = true` to `crates/tracel-experiment/Cargo.toml`.

## Out of scope

- Removing `install_ctrlc` from any external example/CLI-codegen repos (separate `cli/` repo).
- `tracel-runtime::Executor::run()` (deprecated).
