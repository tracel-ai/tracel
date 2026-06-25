# Design: Migrate sdk-example into SDK repo as `examples/mnist-cli`

**Date:** 2026-06-25

## Goal

Move the standalone `sdk-example` repository into the `sdk` repo under an `examples/` directory, following the same structure Burn uses for its examples.

## Context

- `sdk-example` is a separate repo containing an MNIST training demo that exercises the Tracel SDK CLI (via `tracel-app`).
- Burn co-locates all examples inside the main repo under `examples/*`, each as a workspace member with `publish = false`.
- Keeping the example outside the SDK repo creates friction: dependency versions drift, CI doesn't cover it, and contributors have to clone two repos.

## Target Structure

```
sdk/
├── Cargo.toml                        ← add "examples/*" to members; add burn, rand, ctrlc, dotenvy to [workspace.dependencies]
├── crates/
│   └── ...
└── examples/
    └── mnist-cli/
        ├── Cargo.toml                ← publish=false, all deps via workspace, lints.workspace=true
        ├── tracel.toml               ← moved from sdk-example root
        ├── .env                      ← moved from sdk-example root (debug purposes)
        ├── src/
        │   ├── lib.rs
        │   ├── data.rs
        │   ├── model.rs
        │   └── training.rs
        └── examples/
            └── mnist-cli.rs          ← content of sdk-example/src/main.rs
```

Run with: `cargo run --example mnist-cli -p mnist-cli`

## Approach

**Full workspace integration** — mirrors the Burn pattern exactly.

- `"examples/*"` added to `[workspace] members` in SDK root `Cargo.toml`
- Example-specific deps (`burn`, `rand`, `ctrlc`, `dotenvy`) promoted to `[workspace.dependencies]` in root
- All deps in `examples/mnist-cli/Cargo.toml` use `dep.workspace = true`
- `tracel` dep already in workspace as a path to `crates/tracel` — no change needed

## File Movements

| Source (sdk-example) | Destination (sdk) |
|---|---|
| `src/main.rs` | `examples/mnist-cli/examples/mnist-cli.rs` |
| `src/lib.rs` | `examples/mnist-cli/src/lib.rs` |
| `src/data.rs` | `examples/mnist-cli/src/data.rs` |
| `src/model.rs` | `examples/mnist-cli/src/model.rs` |
| `src/training.rs` | `examples/mnist-cli/src/training.rs` |
| `tracel.toml` | `examples/mnist-cli/tracel.toml` |
| `.env` | `examples/mnist-cli/.env` |

## Cargo.toml Changes

### SDK root `Cargo.toml`

Add to `[workspace] members`:
```toml
members = ["crates/*", "examples/*", "xtask"]
```

Add to `[workspace.dependencies]`:
```toml
burn = { git = "...", rev = "...", default-features = false, features = ["train"] }
rand = "..."
ctrlc = "..."
dotenvy = "..."
```

### `examples/mnist-cli/Cargo.toml`

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
burn = { workspace = true, features = ["train", "vision", "metrics", "std", "fusion", "flex", "autotune", "optim"] }
tracel = { workspace = true }
anyhow.workspace = true
log.workspace = true
rand.workspace = true
serde.workspace = true
serde_json.workspace = true
clap.workspace = true
tracing.workspace = true
ctrlc.workspace = true
dotenvy.workspace = true
```

## `.gitignore`

Add `.env` to the SDK repo's `.gitignore` (or the example-level `.gitignore`) so the debug env file is not accidentally committed.

## Out of Scope

- The `sdk-example` repo itself is not deleted — that is a separate decision.
- No changes to CI/CD pipelines (can be done as a follow-up once the example compiles in the workspace).
- No changes to the example's logic (`data.rs`, `model.rs`, `training.rs`).
