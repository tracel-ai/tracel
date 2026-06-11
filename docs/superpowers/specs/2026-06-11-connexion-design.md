# Connexion: single entry point for Context construction

## Problem

`Context` (`crates/tracel-core/src/context.rs`) currently exposes three separate
constructors — `cloud()`, `local(path)`, `station(url)` (feature-gated) — each
directly building the matching backend struct (`CloudBackend`, `LocalBackend`,
`StationBackend`) and wrapping it in `Arc<dyn ExperimentProvider>`.

The desired public API is a single entry point:

```rust
Context::new(Connexion::cloud())?
    .experiment()
    .create("mnist training", |session: &ExperimentRun, config: MnistTrainingConfig| {
        training::run_manual(session, config, devices: vec![Device::autodiff(FlexDevice::into())])
    })?
    .run(MnistTrainingConfig::default())?;
```

`Connexion` is the new object describing *which backend to connect to*, sitting
between `Context::new` and the existing module API (`.experiment()`, etc.).

## Goals

- Single `Context::new(connexion: Connexion)` constructor replacing
  `Context::cloud()` / `Context::local(path)` / `Context::station(url)`.
- `Connexion` enum with one variant per backend: `Cloud`, `None(PathBuf)` (local),
  `Station(Url)` (feature-gated behind `station`).
- Ergonomic constructors: `Connexion::cloud()`, `Connexion::none(path)`,
  `Connexion::station(url)` — call-site boilerplate stays as minimal as today's
  `Context::cloud()` / `Context::local(path)` / `Context::station(url)`.
- Preserve the existing `Arc<dyn ExperimentProvider>` dispatch architecture
  (see `project-context-provider-architecture` memory) — no per-method
  match/dispatch reintroduced.

## Non-goals

- **No per-variant config structs** (`CloudConnexion`, `LocalConnexion`,
  `StationConnexion`, etc.). The backend constructors already encapsulate their
  own configuration (`CloudBackend::create_context()` reads env vars /
  `tracel.toml`; `LocalBackend::create_context(path)` takes a path;
  `StationBackend::create_context(url)` takes a URL). Wrapping these in an
  extra struct per variant would add a layer with no current benefit, given no
  new backends are planned for the foreseeable future.
- **No `Connexion::Custom(...)` escape hatch** for pluggable/third-party
  providers. Not needed at this time.
- **No deprecated wrapper constructors** for the old `Context::cloud()` /
  `local()` / `station()`. `Context` was introduced in commit `432e23a`,
  postdating the `v0.6.0` tag, and has never shipped on crates.io — removing
  these constructors is not a SemVer-breaking change. A repo-wide search
  (`rg "Context::(cloud|local|station)\("`) found no call sites.

## Design

### `Connexion` enum (new file: `crates/tracel-core/src/connexion.rs`)

```rust
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "station")]
use url::Url;

use crate::backend::cloud::{CloudBackend, CloudError};
use crate::backend::local::LocalBackend;
#[cfg(feature = "station")]
use crate::backend::station::StationBackend;
use tracel_experiment::ExperimentProvider;

#[derive(Debug, Clone)]
pub enum Connexion {
    Cloud,
    None(PathBuf),
    #[cfg(feature = "station")]
    Station(Url),
}

impl Connexion {
    pub fn cloud() -> Self {
        Self::Cloud
    }

    pub fn none(path: impl Into<PathBuf>) -> Self {
        Self::None(path.into())
    }

    #[cfg(feature = "station")]
    pub fn station(url: Url) -> Self {
        Self::Station(url)
    }

    pub(crate) fn into_provider(self) -> Result<Arc<dyn ExperimentProvider>, ContextError> {
        match self {
            Connexion::Cloud => Ok(Arc::new(CloudBackend::create_context()?)),
            Connexion::None(path) => Ok(Arc::new(LocalBackend::create_context(path))),
            #[cfg(feature = "station")]
            Connexion::Station(url) => Ok(Arc::new(StationBackend::create_context(url))),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error(transparent)]
    Cloud(#[from] CloudError),
}
```

The `match` inside `into_provider` is a **single, construction-time-only**
dispatch point: it picks which concrete backend struct to box as
`Arc<dyn ExperimentProvider>`. This is different from the pre-refactor
`Backend` enum, where `Context` implemented `ExperimentProvider` itself via a
manual `match` *repeated for every trait method* (one match per operation,
duplicated across the whole trait). That pattern was removed in `432e23a` for
good reason. Here the match exists exactly once; everything after construction
goes through dynamic dispatch on `Arc<dyn ExperimentProvider>` as it does
today.

`Connexion` derives `Debug` and `Clone` — something `Context` itself cannot do
because of `Arc<dyn ExperimentProvider>` — which is useful for logging the
chosen connection before `Context::new` is called.

**Naming note:** `None` was chosen for thematic consistency with
`Cloud`/`Station` ("no remote connection" = local-only storage), with the
known trade-off that it visually overlaps with `Option::None`. The variant may
be renamed later (e.g. to `Local` or `Offline`); nothing else in this design
depends on the specific name.

### `Context::new` (`crates/tracel-core/src/context.rs`, simplified)

```rust
use std::sync::Arc;

use crate::connexion::{Connexion, ContextError};
use tracel_experiment::ExperimentModule;
use tracel_experiment::ExperimentProvider;

#[derive(Clone)]
pub struct Context {
    experiment_provider: Arc<dyn ExperimentProvider>,
}

impl Context {
    pub fn new(connexion: Connexion) -> Result<Self, ContextError> {
        Ok(Self {
            experiment_provider: connexion.into_provider()?,
        })
    }

    pub fn experiment(&self) -> ExperimentModule {
        ExperimentModule::new(self.experiment_provider.clone())
    }
}
```

`context.rs` no longer imports `CloudBackend` / `CloudError` / `LocalBackend`
/ `StationBackend` directly — only `Connexion` and `ContextError` from the new
`connexion` module. `cloud()`, `local(path)`, and `station(url)` are removed
entirely.

### Exports (`crates/tracel-core/src/lib.rs`)

```rust
mod backend;
mod connexion;
mod context;

pub mod experiment;

pub use connexion::{Connexion, ContextError};
pub use context::Context;
```

## Migration

No existing call sites of `Context::cloud()` / `Context::local()` /
`Context::station()` were found in the repo. No migration of downstream code
is needed.

## Open follow-ups (out of scope for this change)

- If a future module beyond `.experiment()` only applies to some `Connexion`
  variants, `Context` may need additional `Option<Arc<dyn XxxProvider>>`
  fields populated per-variant in `into_provider`/`new`. Separate design
  discussion when that need arises.
- `Connexion::None` may be renamed later (collision with `Option::None`).
