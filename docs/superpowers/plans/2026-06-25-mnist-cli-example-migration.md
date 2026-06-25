# mnist-cli Example Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move `sdk-example` into the SDK workspace as `examples/mnist-cli`, following Burn's example structure (lib in `src/`, entry point in `examples/mnist-cli.rs`).

**Architecture:** The SDK root workspace gains an `examples/*` member glob. A new `mnist-cli` crate lives at `examples/mnist-cli/`, using workspace deps throughout. Source files are copied verbatim from `sdk-example/src/`; the binary entry point moves from `src/main.rs` to `examples/mnist-cli.rs`.

**Tech Stack:** Rust / Cargo workspaces, Burn (ML framework via git dep), Tracel SDK crates.

## Global Constraints

- All dependency versions must match what is already declared in the SDK workspace or sdk-example exactly — do not bump any version.
- `publish = false` on the example crate.
- `lints.workspace = true` on the example crate.
- Do not modify any source logic in `data.rs`, `model.rs`, or `training.rs`.
- Working directory for all commands: `/Users/davidmosquera/Documents/Programmation/tracel/sdk`

---

## File Map

| Action | Path |
|---|---|
| Modify | `Cargo.toml` |
| Modify | `.gitignore` |
| Create | `examples/mnist-cli/Cargo.toml` |
| Create | `examples/mnist-cli/tracel.toml` |
| Create | `examples/mnist-cli/.env` |
| Create | `examples/mnist-cli/src/lib.rs` |
| Create | `examples/mnist-cli/src/data.rs` |
| Create | `examples/mnist-cli/src/model.rs` |
| Create | `examples/mnist-cli/src/training.rs` |
| Create | `examples/mnist-cli/examples/mnist-cli.rs` |

---

### Task 1: Update SDK root `Cargo.toml`

Add `examples/*` to workspace members and promote three missing deps (`tracel`, `rand`, `ctrlc`, `dotenvy`) to `[workspace.dependencies]`.

**Files:**
- Modify: `Cargo.toml`

**Interfaces:**
- Produces: workspace dep entries for `tracel`, `rand`, `ctrlc`, `dotenvy` — consumed by Task 2's `Cargo.toml`

- [ ] **Step 1: Create the `examples/` directory so the workspace glob resolves**

```bash
mkdir -p examples/mnist-cli/src examples/mnist-cli/examples
```

- [ ] **Step 2: Add `examples/*` to workspace members**

In `Cargo.toml`, change line 7 from:
```toml
members = ["crates/*", "xtask"]
```
to:
```toml
members = ["crates/*", "examples/*", "xtask"]
```

- [ ] **Step 3: Add missing workspace dependencies**

After line 74 (`tracel-core = ...`), add:
```toml
tracel = { path = "crates/tracel", version = "0.6.0" }
```

After line 23 (`]` closing burn features), add:
```toml
rand = "0.9.2"
ctrlc = "3.5.2"
dotenvy = "0.15"
```

The `[workspace.dependencies]` block should look like:
```toml
[workspace.dependencies]
burn = { git = "https://github.com/tracel-ai/burn", rev = "35ff68bd3b2faa6b6f651e5d7ab5939b4504f799", default-features = false, features = [
    "train",
] }

rand = "0.9.2"
ctrlc = "3.5.2"
dotenvy = "0.15"

url = "2.5.8"
# ... rest of deps unchanged ...

## Crate
tracel = { path = "crates/tracel", version = "0.6.0" }
tracel-app = { path = "crates/tracel-app", version = "0.6.0" }
# ... rest unchanged ...
```

- [ ] **Step 4: Verify workspace resolves**

```bash
cargo metadata --no-deps --format-version 1 | python3 -c "import sys,json; ws=json.load(sys.stdin); print([p['name'] for p in ws['packages']])"
```

Expected: output includes `"mnist-cli"` once the crate is created in Task 2 (currently it will error if `examples/` dir doesn't exist yet — that's fine, just verify no syntax errors with):
```bash
cargo check --workspace 2>&1 | grep "^error\[" | head -5
```

Expected: no `error[E...]` lines (warnings OK).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml
git commit -m "chore: add examples/* to workspace members and promote shared deps"
```

---

### Task 2: Scaffold `examples/mnist-cli/`

Create the crate directory, `Cargo.toml`, `tracel.toml`, and `.env`. No Rust source yet.

**Files:**
- Create: `examples/mnist-cli/Cargo.toml`
- Create: `examples/mnist-cli/tracel.toml`
- Create: `examples/mnist-cli/.env`
- Modify: `.gitignore`

**Interfaces:**
- Consumes: workspace dep entries for `tracel`, `rand`, `ctrlc`, `dotenvy` from Task 1
- Produces: compilable crate skeleton (no lib or binary yet, compilation will fail until Task 3)

- [ ] **Step 1: Create `examples/mnist-cli/Cargo.toml`**

```toml
[package]
name = "mnist-cli"
edition.workspace = true
version.workspace = true
publish = false

[lints]
workspace = true

[features]
default = ["wgpu", "flex"]
flex = ["burn/flex"]
wgpu = ["burn/wgpu"]
cuda = ["burn/cuda"]

[dependencies]
burn = { workspace = true, features = [
    "vision",
    "metrics",
    "std",
    "fusion",
    "autotune",
    "optim",
] }
tracel = { workspace = true }
anyhow.workspace = true
clap.workspace = true
ctrlc.workspace = true
dotenvy.workspace = true
log.workspace = true
rand.workspace = true
serde = { workspace = true, features = ["std"] }
serde_json.workspace = true
tracing.workspace = true
```

- [ ] **Step 2: Create `examples/mnist-cli/tracel.toml`**

```toml
name = "burn-central-example"
owner = "test"
```

- [ ] **Step 3: Create `examples/mnist-cli/.env`**

Copy verbatim from `sdk-example/.env`:
```
BURN_CENTRAL_API_KEY=380f2206-6074-4ac3-83d9-a14f497a560e # for sdk-integration-example
TRACEL_NAMESPACE=test
TRACEL_PROJECT=burn-central-example
TRACEL_ENV=Development
```

- [ ] **Step 4: Add `.env` to SDK `.gitignore`**

Append to `.gitignore`:
```
examples/**/.env
```

- [ ] **Step 5: Commit scaffold**

```bash
git add examples/mnist-cli/Cargo.toml examples/mnist-cli/tracel.toml .gitignore
git commit -m "chore: scaffold examples/mnist-cli crate"
```

---

### Task 3: Copy source files

Copy the four Rust source files verbatim from `sdk-example/src/` into `examples/mnist-cli/src/`.

**Files:**
- Create: `examples/mnist-cli/src/lib.rs`
- Create: `examples/mnist-cli/src/data.rs`
- Create: `examples/mnist-cli/src/model.rs`
- Create: `examples/mnist-cli/src/training.rs`

**Interfaces:**
- Produces: `pub mod data`, `pub mod model`, `pub mod training` from `lib.rs` — consumed by Task 4's entry point

- [ ] **Step 1: Copy source files**

```bash
cp /Users/davidmosquera/Documents/Programmation/tracel/sdk-example/src/lib.rs examples/mnist-cli/src/lib.rs
cp /Users/davidmosquera/Documents/Programmation/tracel/sdk-example/src/data.rs examples/mnist-cli/src/data.rs
cp /Users/davidmosquera/Documents/Programmation/tracel/sdk-example/src/model.rs examples/mnist-cli/src/model.rs
cp /Users/davidmosquera/Documents/Programmation/tracel/sdk-example/src/training.rs examples/mnist-cli/src/training.rs
```

- [ ] **Step 2: Verify files landed correctly**

```bash
ls examples/mnist-cli/src/
```

Expected output:
```
data.rs  lib.rs  model.rs  training.rs
```

- [ ] **Step 3: Check crate compiles (no entry point yet — expect "no lib target" warning, not errors)**

```bash
cargo check -p mnist-cli 2>&1 | grep "^error" | head -10
```

Expected: no `error` lines. If there are errors, they will be import resolution errors — check that `tracel`, `burn`, and `rand` are found in the workspace deps added in Task 1.

- [ ] **Step 4: Commit source files**

```bash
git add examples/mnist-cli/src/
git commit -m "feat: add mnist-cli library source files"
```

---

### Task 4: Create the example entry point

Move `sdk-example/src/main.rs` content to `examples/mnist-cli/examples/mnist-cli.rs`, updating the crate import from `tracel_example::` to `mnist_cli::`.

**Files:**
- Create: `examples/mnist-cli/examples/mnist-cli.rs`

**Interfaces:**
- Consumes: `mnist_cli::training::{self, MnistTrainingConfig}` from Task 3's `training.rs`
- Consumes: `tracel::cli::{Cli, JsonMapper}`, `tracel::experiment::ExperimentRun`, `tracel::{Connection, Context}`
- Produces: runnable example via `cargo run --example mnist-cli -p mnist-cli`

- [ ] **Step 1: Create `examples/mnist-cli/examples/mnist-cli.rs`**

The original `main.rs` imports `tracel_example::training` — update to `mnist_cli::training`:

```rust
#![recursion_limit = "256"]

use burn::backend::{FlexDevice, wgpu::WgpuDevice};
use burn::tensor::Device;
use mnist_cli::training::{self, MnistTrainingConfig};

use tracel::cli::{Cli, JsonMapper};
use tracel::experiment::ExperimentRun;
use tracel::{Connection, Context};

fn main() -> anyhow::Result<()> {
    let module = Context::new(Connection::Cloud)?.experiment();
    let job = module.create("mnist_flex", |session: &ExperimentRun, config| {
        training::run(
            session,
            config,
            vec![Device::autodiff(WgpuDevice::default().into())],
        )
    });
    let default_job = module.create("mnist_wgpu", |session: &ExperimentRun, config| {
        training::run(
            session,
            config,
            vec![Device::autodiff(FlexDevice::default().into())],
        )
    });

    Cli::new()
        .register(
            job,
            JsonMapper::with_default(MnistTrainingConfig::default()),
        )
        .default_job(default_job, MnistTrainingConfig::small())
        .run()?;

    Ok(())
}
```

- [ ] **Step 2: Check the example compiles**

```bash
cargo check --example mnist-cli -p mnist-cli 2>&1 | grep "^error" | head -20
```

Expected: no `error` lines. The most likely failure is `mnist_cli::training` not found — that means the crate name resolution failed; verify the package `name = "mnist-cli"` in `examples/mnist-cli/Cargo.toml` (Rust converts `-` to `_` in module names automatically).

- [ ] **Step 3: Commit the entry point**

```bash
git add examples/mnist-cli/examples/mnist-cli.rs
git commit -m "feat: add mnist-cli example entry point"
```

---

### Task 5: Final verification

Confirm the example is a proper workspace member, compiles cleanly, and the `.env` is gitignored.

**Files:** none new

- [ ] **Step 1: Verify `mnist-cli` is in the workspace**

```bash
cargo metadata --no-deps --format-version 1 | python3 -c "import sys,json; ws=json.load(sys.stdin); print([p['name'] for p in ws['packages'] if 'mnist' in p['name']])"
```

Expected: `['mnist-cli']`

- [ ] **Step 2: Full workspace check**

```bash
cargo check --workspace 2>&1 | grep "^error" | head -20
```

Expected: no `error` lines.

- [ ] **Step 3: Verify `.env` is gitignored**

```bash
git status --short examples/mnist-cli/.env
```

Expected: no output (file is ignored). If it appears, check that `examples/**/.env` is in `.gitignore`.

- [ ] **Step 4: Final commit**

```bash
git add .gitignore
git commit -m "chore: ensure examples .env files are gitignored"
```

If `.gitignore` was already committed in Task 2 with the right content, skip this step.
