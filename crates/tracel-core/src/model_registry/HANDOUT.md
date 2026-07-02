# Handout: model_registry code review follow-ups

Context for picking this back up in another conversation/branch.

From a multi-angle review of `crates/tracel-core/` on `feat/add_model_registry`.
No correctness bugs found — these are the 4 quality issues that survived
verification, ranked most severe first.

## 1. `Context::model_registry()` rebuilds an HTTP client on every call

- Where: `context.rs:27-31`
- `ModelRegistryModule::new` (`mod.rs:251-257`) does
  `transfer_client: ReqwestTransferClient::new()`, which calls
  `reqwest::blocking::Client::new()` (`tracel-artifact/src/transfer.rs:30-34`).
  `reqwest::blocking` spins up its own background Tokio runtime + connector/TLS
  setup under the hood — not free.
- `Context::experiment()` (`context.rs:23-25`) looks identical in shape but
  is actually free, because `ExperimentModule` only holds an `Arc` clone of a
  provider that was already constructed once in `Context::new()`
  (`tracel-experiment/src/provider.rs:33-41`). `ModelRegistryModule` also owns
  a *second* thing, the transfer client, and that part gets rebuilt from
  scratch every call.
- Fix: construct the `ModelRegistryModule` (or at least the transfer client)
  once in `Context::new()`/`Context` and clone it out of `model_registry()`,
  instead of building fresh in the method body.

## 2. Removed `create_cloud_experiment_run` — external breakage risk

- Where: `experiment/mod.rs` (removed re-export) and
  `experiment/remote/cloud/mod.rs:247` (removed function), from commit
  `c01c1c0 "refactor: delete unused reexport"`.
- The removed function's comment said "temporary re-export for the runtime
  crate, will be erased when we detach ourself completely from runtime" — that
  `runtime` crate isn't part of this workspace (no member named `runtime`
  anywhere), so it can't be checked from here.
- grep confirms zero remaining references anywhere in this repo, so it's safe
  *locally*, but if an external/sibling `runtime` crate depends on
  `tracel_core::experiment::create_cloud_experiment_run` via crates.io/git,
  this is a breaking change for its build.
- Action: confirm with whoever owns the runtime crate (or check its source
  directly) before/after this ships.

## 3. `ModelRegistryModule<FTC>` generic is test-only

- Where: `mod.rs:246-269` (the struct + `with_transfer_client` ctor).
- `with_transfer_client` is only ever called from this module's own
  `#[cfg(test)]` block. The only production caller,
  `Context::model_registry()`, always goes through `ModelRegistryModule::new`
  (`FTC = ReqwestTransferClient`). The type is also re-exported unparameterized
  from `tracel-core::lib.rs`/`tracel::lib.rs`, so external callers can't use
  the generic meaningfully either.
- Inconsistent with the sibling `ExperimentModule`, which has exactly one
  non-generic constructor (`tracel-experiment/src/provider.rs:37-41`).
- Fix: make `ModelRegistryModule` concrete over `ReqwestTransferClient`; if
  test injectability is still wanted, make `download_to` itself generic over
  `FTC: FileTransferClient` instead of the whole struct
  (`download_artifacts_to_sink_with_client` is already generic, so no new
  abstraction needed).

## 4. Presigned-file → `ArtifactDownloadFile` mapping duplicated 4x

- Where: `model_registry/cloud.rs:31-48`, `model_registry/station.rs:31-45`,
  plus the pre-existing `experiment/remote/cloud/mod.rs` and
  `experiment/remote/station/mod.rs` download loops.
- All four hand-roll the same
  `.into_iter().map(|f| ArtifactDownloadFile { rel_path: f.rel_path, url: f.url, ... }).collect()`
  shape. `tracel-artifact/src/download.rs` has no `From`/helper for this, so
  each backend integration re-derives it.
- Fix: a small local helper, or a `From<PresignedFileEntry> for
  ArtifactDownloadFile` impl per response type, to collapse at least the two
  new sites (and ideally the two pre-existing ones too).
