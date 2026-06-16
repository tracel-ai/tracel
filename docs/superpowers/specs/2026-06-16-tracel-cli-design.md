# tracel-cli — Design Spec

**Date:** 2026-06-16
**Branch:** feat/add_cli_registers
**Status:** Approved

---

## Goal

Add a `tracel-cli` crate that lets users register experiment jobs and select which one to run at launch time via CLI arguments, without changing any existing domain crates.

```
cargo run -- "mnist" "small"
```

---

## Layer model

```
tracel-cli          ← new crate (CLI concern)
  └─ tracel-experiment  ← domain crate, unchanged
       └─ tracel-core   ← unchanged
```

`tracel-cli` is the only crate that changes. It depends on `tracel-experiment` to access `ExperimentJob`. Domain crates (`tracel-experiment`, `tracel-core`) have no knowledge of the CLI.

---

## The `CliJob` trait

Defined in `tracel-cli`. This is a CLI-layer abstraction only — domain crates never see it.

```rust
pub trait CliJob<I, O> {
    fn execute(&self, input: I) -> Result<O, Box<dyn Error + Send + Sync>>;
}
```

`tracel-cli` provides an implementation for `ExperimentJob`:

```rust
impl<I, O> CliJob<I, O> for ExperimentJob<I, O> {
    fn execute(&self, input: I) -> Result<O, Box<dyn Error + Send + Sync>> {
        self.run(input)
    }
}
```

When `InferenceJob` is added to `tracel-inference` in the future, a matching impl is added in `tracel-cli` the same way.

---

## `Cli` builder API

```rust
pub struct Cli { ... }

impl Cli {
    pub fn new() -> Self

    /// Register a job with a config-mapper closure.
    /// Accepts anything implementing CliJob<I, O>.
    pub fn register<J, I, O, F>(self, name: &str, job: J, mapper: F) -> Self
    where
        J: CliJob<I, O> + 'static,
        F: Fn(&str) -> Result<I, Box<dyn Error + Send + Sync>> + 'static,
        I: 'static,
        O: 'static,

    /// Convenience wrapper for ExperimentJob. Calls register() internally.
    pub fn register_exp<I, O, F>(self, name: &str, job: ExperimentJob<I, O>, mapper: F) -> Self
    where
        F: Fn(&str) -> Result<I, Box<dyn Error + Send + Sync>> + 'static,
        I: 'static,
        O: 'static,

    /// Set the job that runs when no job name is given on the CLI.
    pub fn default(self, name: &str) -> Self

    /// Parse args and dispatch. Exits with an error message on bad input.
    pub fn run(self) -> Result<(), CliError>
}
```

### User code example

```rust
fn main() {
    let module = Context::new(Connection::Cloud).experiment();
    let job1 = module.create("mnist", train_mnist);
    let job2 = module.create("vgg", train_vgg);

    Cli::new()
        .register_exp("mnist", job1, |cfg| match cfg {
            "small" => Ok(MnistConfig::small()),
            "large" => Ok(MnistConfig::large()),
            _       => Err(format!("unknown config: {cfg}").into()),
        })
        .register_exp("vgg", job2, |cfg| match cfg {
            "v1" => Ok(VggConfig::v1()),
            _    => Err(format!("unknown config: {cfg}").into()),
        })
        .default("mnist")
        .run()
        .unwrap();
}
```

---

## Arg parsing rules

| Invocation | Behaviour |
|---|---|
| `cargo run -- "mnist" "small"` | Run job `mnist` with config `"small"` |
| `cargo run -- "small"` | Run default job with config `"small"` (error if no default set) |
| `cargo run` | Do nothing (config is mandatory — even with a default, no job runs) |
| Unknown job name | Print available job names, return `CliError` |
| Mapper returns `Err` | Print mapper error, return `CliError` |
| `job.execute` returns `Err` | Print error, return `CliError` |
| 3+ args | Print usage, return `CliError` |

Config string is always mandatory. `cargo run` alone never triggers execution.

---

## Internal structure

### Type erasure in `register`

`register` immediately erases `J`, `I`, `O` into a single `Box<dyn Fn(&str) -> Result<(), Box<dyn Error + Send + Sync>>>` by composing `mapper` and `job.execute`:

```rust
let erased = Box::new(move |config_str: &str| {
    let input = mapper(config_str)?;
    job.execute(input).map(|_| ())
});
self.jobs.insert(name.to_string(), erased);
```

### `Cli` struct fields

```rust
struct Cli {
    jobs: HashMap<String, Box<dyn Fn(&str) -> Result<(), Box<dyn Error + Send + Sync>>>>,
    default: Option<String>,
}
```

---

## Error type

```rust
pub enum CliError {
    UnknownJob { name: String, available: Vec<String> },
    MissingDefault,
    ConfigError(Box<dyn Error + Send + Sync>),
    JobError(Box<dyn Error + Send + Sync>),
    TooManyArgs,
}
```

---

## File layout

```
crates/tracel-cli/src/
  lib.rs      — re-exports Cli, CliError, CliJob
  cli.rs      — Cli struct, builder methods, run(), arg parsing
  error.rs    — CliError enum + Display impl
  job.rs      — CliJob trait + impl for ExperimentJob
```

---

## Out of scope (first iteration)

- `--help` / `--list` flags
- Listing available configs per job
- `InferenceJob` support (trait is ready for it, impl deferred)
- Async job execution
