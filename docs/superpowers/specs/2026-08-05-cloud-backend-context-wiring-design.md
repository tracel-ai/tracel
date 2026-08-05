# Cloud backend, context, and project creation — public API wiring

## Problem

`tracel-core`'s `backend` module (`crates/tracel-core/src/backend/`) is declared as `mod backend;` in
[`lib.rs`](../../../crates/tracel-core/src/lib.rs) with no `pub use` re-exporting anything from it. As a
result, `CloudBackend`, `StationBackend`, `LocalBackend`, `AuthMethod`, `CloudSession`, and `authenticate`
all compile but are unreachable from outside the crate. There is no public entry point at all.

This regressed when the `Connection` enum (the previous public entry point, `pub use connection::{Connection,
ContextError}`) was deleted in commit `7c0ab51` ("add IntoPovider trait and remove connection") and never
replaced. `StationBackend::new` is `pub fn`, but `CloudBackend::new` and `LocalBackend::new` are private
`fn`, an inconsistency left over from the same change.

There is no caller today (CLI, `tracel` crate re-export, or otherwise) depending on this — this design
establishes the public surface before one exists.

## Two independent flows

The design treats "build a `Context`" and "create a cloud project" as genuinely separate operations that
should not share a construction path:

1. **Build a `Context`** — for running experiments/inference/model-registry/dataset operations. Requires a
   fully-formed backend. For `CloudBackend` specifically, that means a resolved namespace and project.
2. **Create a cloud project** — requires only an authenticated session. Deliberately has no namespace/project
   dependency (you don't have a project yet — you're creating one), and never touches `Context`.

Forcing these through one constructor was the original bug: `CloudBackend::new` used to require a resolved
namespace/project before `create_project` could even be called, which is a chicken-and-egg problem when the
project doesn't exist yet.

## Public API surface

Flat re-exports at the `tracel-core` crate root, matching the existing pattern for `dataset` and
`model_registry` in `lib.rs` (as opposed to a `pub mod backend` namespace):

```
pub use backend::cloud::{authenticate, AuthMethod, CloudBackend, CloudError, CloudSession};
pub use backend::station::{StationBackend, StationError};
pub use backend::local::LocalBackend;
```

Exact re-export list to be finalized during implementation (e.g. whether `LocalBackend` needs its own error
type).

## Constructor visibility

`CloudBackend::new` and `LocalBackend::new` become `pub fn`, matching `StationBackend::new`. `LocalBackend::new`
also currently just wraps a `PathBuf` with no validation — flagged as a follow-up, not blocking this design.

## Flow 1 — Context construction

```rust
CloudBackend::new(AuthMethod) -> Result<CloudBackend, CloudError>
StationBackend::new(Url)      -> Result<StationBackend, StationError>
LocalBackend::new(PathBuf)    -> LocalBackend

Context::new(backend) -> Result<Context, ContextError>   // via IntoProviders, unchanged
```

`Context::new(backend)` (generic over `impl IntoProviders`) remains the single path into `Context`. No
`.context()` convenience method on `IntoProviders` — YAGNI; `Context::new(backend)` is not onerous enough to
warrant the extra surface area. `IntoProviders` itself is unchanged; it was never the problem, visibility was.

Example:

```rust
use tracel_core::{AuthMethod, CloudBackend, Context};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let backend = CloudBackend::new(AuthMethod::Env)?;
    let ctx = Context::new(backend)?;
    Ok(())
}
```

## Flow 2 — Cloud project creation

```rust
authenticate(AuthMethod) -> Result<CloudSession, CloudError>
session.create_project(owner, name, description) -> Result<(), CloudError>   // &self, session stays reusable
```

`CloudSession` wraps `tracel_client::Client` privately — the raw `Client` type never appears in a public
signature. `create_project` takes `&self` and returns `()`: creating a project is a complete, standalone
operation that does not imply the caller wants a `Context` or is willing to pay for building one (cache dir
resolution, `ModelCache` construction). Because it borrows rather than consumes, one authenticated session
can create multiple projects.

Example:

```rust
use tracel_core::{authenticate, AuthMethod};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session = authenticate(AuthMethod::Env)?;
    session.create_project("my-org", "my-project", "a description")?;
    Ok(())
}
```

## Bridging the two flows: `CloudBackend::from_session`

The gap this design closes: creating a project and then immediately building a `Context` scoped to it, in the
same process, without a wasted re-authentication and without relying on env vars/`tracel.toml` coincidentally
matching the project just created.

```rust
impl CloudBackend {
    pub fn from_session(
        session: CloudSession,
        namespace: String,
        project: String,
    ) -> Result<Self, CloudError> {
        let client = session.into_client();

        let cache_root = crate::resolve_cache_dir()
            .ok_or(CloudError::NoCacheDir)?
            .join("cloud")
            .join(&namespace)
            .join(&project)
            .join("models");

        Ok(Self {
            client,
            namespace,
            project,
            file_transfer_client: ReqwestTransferClient::new(),
            model_cache: ModelCache::new(cache_root),
        })
    }

    pub fn new(authentication: AuthMethod) -> Result<Self, CloudError> {
        let session = authenticate(authentication)?;
        let (namespace, project) = discover_namespace_project()?;
        Self::from_session(session, namespace, project)
    }
}
```

`new` becomes a thin, discovery-based wrapper over `from_session` — no duplicated struct-building logic
between the two paths.

### Why `from_session` over having `create_project` return a `CloudBackend` directly

An alternative considered: `create_project(self, ...) -> Result<CloudBackend, CloudError>`, consuming the
session and handing back a ready-to-use backend in one call, avoiding the need to pass `owner`/`name` a second
time.

Rejected because it violates single responsibility: `create_project`'s name promises "create a project," and
having it also always resolve a cache dir and construct a `ModelCache` is a second, unrelated job bundled in
silently. A caller who only wants to create a project (e.g. a `tracel project create` CLI command that exits
immediately after) would pay for backend construction it never uses, and would lose the ability to create
multiple projects off one session since `self` would be consumed. The minor cost of `from_session` — passing
`owner`/`name` twice — is an acceptable, explicit tradeoff for keeping "create a project" and "build a backend
scoped to a project" as separate, independently useful operations.

### Combined usage — create then use, same process

```rust
use tracel_core::{authenticate, AuthMethod, CloudBackend, Context};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session = authenticate(AuthMethod::Env)?;
    session.create_project("my-org", "my-project", "a description")?;

    let backend = CloudBackend::from_session(session, "my-org".to_string(), "my-project".to_string())?;
    let ctx = Context::new(backend)?;

    Ok(())
}
```

## Error handling

No changes needed. `CloudError`, `StationError`, and `ContextError` already model the relevant failure modes
(`NoNamespace`, `NoProject`, `NoCacheDir`, `InvalidCredentials`, etc.), and `ContextError` already wraps
`CloudError`/`StationError` via `#[from]`.

## Testing

No live network calls. Since there is no caller yet, testing is wiring/visibility verification within
`tracel-core`:

- `authenticate(AuthMethod::ApiKey("..."))` → `CloudSession::create_project(...)` compiles and type-checks
  without touching `Context`, and can be called twice on the same session.
- `CloudBackend::new(AuthMethod::ApiKey("..."))` → `Context::new(backend)` compiles.
- `authenticate(...)` → `create_project(...)` → `CloudBackend::from_session(session, ns, proj)` →
  `Context::new(backend)` compiles as the combined flow.
- All re-exported types (`CloudBackend`, `StationBackend`, `LocalBackend`, `AuthMethod`, `CloudSession`,
  `authenticate`) are reachable as `tracel_core::X` from a test in a separate integration-test crate boundary
  (`tests/`), not just from within `tracel-core` itself — this is what actually proves the visibility fix
  worked, since `mod`-private items are still reachable from unit tests inside the same crate.

## Out of scope

- Whether/how the top-level `tracel` crate (`crates/tracel/src/lib.rs`) re-exports these types. Separate,
  later decision.
- `LocalBackend::new` validation/signature review.
- Any CLI command wiring (`tracel-app`) — no caller exists yet.
