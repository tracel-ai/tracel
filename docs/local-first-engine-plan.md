# Tracel local-first engine + sync — build plan

Status: **draft for review**. This is a working design doc; the "Open questions" section
is the intended refinement surface.

## 1. Goal

Build a local-first MLOps **engine** (`tracel-engine`) and a **sync** layer (`tracel-sync`)
as libraries, plus a **fake harness** that drives them so we can develop and validate the
design before `tracel-chat` exists.

- Consumer target: `tracel-chat` — an egui app (native now, browser-wasm later) that is
  itself a `burn` application. It retrains models and monitors inference locally, and
  (future) synchronizes with the cloud.
- The engine is a **backend behind the SDK's one facade**, never a competing domain API.
- First milestone runs **native only**, against a **simulated remote** (no real cloud
  endpoints, no wasm).

## 2. Decisions already settled (challenge these first)

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | **The SDK is the one facade.** The engine is a backend behind it; the app programs against SDK modules (`experiment/inference/models`). | Cohesion — a single domain vocabulary across cloud/station/engine. |
| D2 | **Reuse domain + application (Path A). The engine lives in `tracel-app` (backend monorepo, formerly `burn-central-app`) as a composition root over the same bounded-context crates as the cloud API and station — minus the HTTP shell.** One shared domain model across cloud/station/engine. | Co-located with the crates it reuses (no cross-repo extraction). A shared model makes sync's bilateral contract shared *code*, not a translation layer. The engine ≈ station without axum. The wasm cost becomes a bounded dependency cleanup (Q7, Q12), not a duplicated domain. |
| D3 | **The SDK stays untainted by sync.** Sync, conflicts, and content-addressing live in `tracel-sync`/engine. Domain verbs = SDK (uniform); backend lifecycle (sync) = engine handle. | Sync only exists for local replicas (engine/station). It is `VACUUM`/replication, not `SELECT`. A cloud-only SDK user never links it. |
| D4 | **Registry = git model.** Immutable, content-addressed versions (grow-only set) + mutable named refs (`production`, `latest`, `v4`). Content-addressing is **engine-internal**; the SDK registry vocabulary is unchanged. | Immutable objects merge trivially; the only real conflict surface is a handful of mutable refs. |
| D5 | **Causal reconciliation.** Refs carry a version vector. Fast-forward auto-applies at any divergence size; true concurrency **surfaces as a conflict** and is never silently clobbered. | Wall-clock LWW silently discards intentional state over long offline windows. |
| D6 | **Two planes.** Metadata (small, structured — reconcile eagerly and fully) vs. blobs (large, content-addressed — transfer lazily, dedup by hash). | Edge devices hold the full catalog cheaply, materialize weights on demand. |
| D7 | **Hub-and-spoke.** Cloud is the reconciliation authority. P2P is not built, but content-addressing keeps it open. | Avoids N-replica distributed-systems complexity now without foreclosing it. |
| D8 | **Sync is per-domain strategies over a thin shared toolkit**, not one generic engine. Each domain's `merge` is a pure function; transport/storage sit behind ports (`SyncTarget`, the local store). | Merge semantics differ fundamentally per domain (registry refs vs. append-only logs vs. annotation CRDTs). Unit-test each merge in isolation; integration-test via the simulator. |
| D9 | **Lineage from day one.** Every version stamped with origin + parent; every ref move carries a version vector. | Causal lineage is un-backfillable — impossible to reconstruct cleanly later. This is the one thing that must exist in the very first store. |
| D10 | **Async is confined to the sync subsystem.** The local data API (read/write) stays sync. | Native has threads + `block_on`; wasm writes buffer to memory + drain via a spawned task. Only sync is inherently async. |
| D11 | **wasm-compatible from the start.** The engine targets `wasm32-unknown-unknown` (tracel-chat's real target) from day one — proven by verticalizing one bounded context end-to-end in Phase 0, not a monorepo-wide refactor up front. Storage diverges by target (wasm OPFS / native SQLite); only `domain`/`application` are shared across targets. | Keeps the design honest — no native-only debt (threads, blocking, `std::fs`) to retrofit later. Forces the storage/clock/blob/transport ports to be real. Makes the dependency surgery (Q12) a Phase-0 task, not deferred. |

## 3. Crate layout

```
tracel  (SDK repo) — PUBLIC client facade; unchanged vocabulary + one seam
  tracel-experiment / tracel-inference / tracel-artifact   ports + primitives + modules
  tracel-core     Context, Cloud/Station backends
     + NEW: Context::from_providers(Providers)             ← only SDK change, sync-agnostic
  (depends on NOTHING in the backend monorepo — stays light + public)

tracel-app  (backend monorepo, formerly burn-central-app) — where the engine + sync live
  domain + application bounded contexts                    reused by ALL composition roots
     experiment · model-registry · dataset · inference-group · shared-*   (cloud API, station,
                                                                           AND the engine)
  per-domain sync strategies (DomainSync impls)            live WITH each domain context (Q11)
  sync toolkit    SyncTarget · blob store · version vectors · cursors · orchestration
  tracel-engine   composition root over domain/application (Path A) + storage adapters
                  (native SQLite like station / wasm OPFS) + implements the SDK provider ports
                  + the sync handle.   ≈ station minus the axum shell.
  burn-station-*  same crates + an HTTP shell → convergence target (station = engine + HTTP)

CONSUMERS
  burn script   → tracel (SDK) + cloud                     never links the engine
  tracel-chat   → tracel (SDK facade) + tracel-app::tracel-engine (in-process local backend)
  fake harness  → same as tracel-chat, facade-only (see §6)
```

Dependency edges (all one-directional; no cycle):
- `tracel-app::tracel-engine` → `tracel::{tracel-experiment, tracel-inference}` (implements the
  SDK ports — a NEW per-crate edge; the rest of the backend does not depend on the SDK).
- `tracel-app::tracel-engine` → its own monorepo's `domain`/`application` crates (local reuse).
- `tracel-chat` → both the SDK and the engine crate.
- The SDK never depends on the backend, so it stays light and public.

**Naming collision to resolve:** a `tracel-app` *crate* already exists in the SDK repo
(`crates/tracel-app`); the `burn-central-app → tracel-app` repo rename will clash in
conversation even if Cargo namespaces keep them distinct. Pick distinct names early.

## 4. Core types & traits (illustrative — shapes, not final)

**SDK seam (`tracel-core`):**
```rust
impl Context {
    // Providers already exists; this just exposes a public constructor over it.
    pub fn from_providers(providers: Providers) -> Self;
}
```

**`tracel-sync` — the shared toolkit (domain-agnostic; no Version/Ref here):**
```rust
struct VersionVector(/* per-replica counters */);   // reusable causal primitive
struct BlobId(Digest);                               // content address (shared blob plane)

trait SyncTarget {   // in-mem hub (tests) | cloud client (later) | peer engine (P2P tests)
    async fn pull(&self, domain: DomainId, since: Cursor) -> WireDelta;
    async fn push(&self, domain: DomainId, delta: WireDelta) -> Ack;
    async fn fetch_blob(&self, id: BlobId) -> ByteStream;
}

// Each DOMAIN implements this; the toolkit only drives it. Delta/Conflict are per-domain.
trait DomainSync {
    type Delta;                                      // what this domain ships
    type Conflict;                                   // often `Never`
    fn local_changes(&self, since: Cursor) -> Self::Delta;
    fn merge(&mut self, remote: Self::Delta) -> Vec<Self::Conflict>;   // pure, per-domain
    fn blob_refs(&self) -> Vec<BlobId>;              // feeds the shared transfer pass
}
```

**Per-domain sync semantics (each strategy lives WITH its domain):**

| Domain | Immutable objects | Mutable / streaming | Merge | Conflict |
|---|---|---|---|---|
| Model registry | versions (content-addressed) | named refs (`production`, VV) | union + ref fast-forward/conflict | ref divergence |
| Experiments | runs, artifacts (content-addressed) | append-only event log per run | union; catch up log by cursor | ~none (single-writer) |
| Datasets (later) | dataset versions (content-addressed) | annotations (multi-writer) | version union + annotation CRDT | annotation edits |
| Inference telemetry | — | append-only samples | aggregate / union | none |

```rust
// registry strategy — owns the Version/Ref types that were WRONGLY in the core before
impl DomainSync for RegistrySync {
    type Delta = RegistryDelta;      // { versions: Vec<Version>, refs: Vec<Ref{ target, vv }> }
    type Conflict = RefConflict;     // { name, local, remote, base }
    // merge = union immutable versions + fast-forward-or-conflict each ref by version vector
}
impl DomainSync for ExperimentSync {
    type Delta = ExperimentDelta;    // { new_runs, events_since_cursor }
    type Conflict = Never;
    // merge = union runs, append events past each run's cursor
}
```

**`tracel-engine` — `sync()` is a thin driver over the strategies:**
```rust
async fn sync(&self, target: &dyn SyncTarget) -> SyncReport {
    // each domain handled with its own concrete Delta/Conflict types (no generic god-delta)
    target.push(REGISTRY, self.registry.local_changes(cursor)).await;
    conflicts.registry = self.registry.merge(target.pull(REGISTRY, cursor).await);
    target.push(EXPERIMENT, self.experiment.local_changes(cursor)).await;
    /* … one arm per domain … */
    transfer_blobs(union_of_all(blob_refs)).await;   // one shared, prioritized pass
    checkpoint();
}
```

**`tracel-engine` — the handle:**
```rust
struct Engine { /* local store, blob store, providers */ }
impl Engine {
    fn open_native(path: &Path) -> Result<Engine>;
    fn context(&self) -> Context;                 // SDK-typed domain API (from_providers under the hood)
    async fn sync(&self, target: &dyn SyncTarget) -> SyncReport;   // backend lifecycle — NOT on the SDK
    fn resolve_conflict(&self, name: &str, choice: VersionId);
    // materialization / status queries live here too (local-replica status, not SDK domain)
}
```

The app holds one `Engine`, gets its **domain API as SDK types** via `engine.context()`, and
uses `engine.sync()` for lifecycle. Two method groups, one handle, domain vocabulary stays SDK.

## 5. Roadmap (two deliverables, then the rest)

### Deliverable 1 — Model registry in the engine (basic model operations), wasm-compatible
Scope: the `model-registry` bounded context only. Reuse its `domain`/`application` (Path A) and:
- **Q12 surgery** on `model-registry` + `shared-domain` (+ the non-axum bits of
  `shared-infrastructure`) so they compile to `wasm32-unknown-unknown`.
- **Storage behind the persistence port:** native = reuse station's SQLite; wasm = new
  OPFS/IndexedDB adapter for metadata **and** blobs (weights are in scope from the start here).
- **In-memory-authoritative metadata** (hydrate at `open()`, sync reads from memory, async persist)
  so the SDK's sync API works on wasm; wasm clock (`web-time`) + `getrandom` js.
- **Blob materialization:** `load` needs weight bytes; on wasm that read is async, so an async
  `materialize(version)` on the engine handle pulls the blob into memory, and the SDK's sync `load`
  then reads materialized bytes. Unused blobs stay catalog-only in OPFS.
- **Content-addressed versions + lineage** (origin/parent) from day one (D9), before sync exists.
- Wire through the engine composition root + `Context::from_providers` + the SDK `ModelRegistryProvider`.

**Milestone:** register / list / load models + versions through the SDK facade, native AND wasm.
*Out: sync, experiments, inference.*

### Deliverable 2 — Model registry sync (console ↔ engine/station)
Make the registry sync between console (hub) and the engine and station (spokes).
- The `model-registry` **`DomainSync` strategy** — immutable content-addressed versions (grow-only
  set) + named refs with version vectors — lives in the `model-registry` crate, shared **as code**
  by the console reconciler (hub) and both spokes (Q11).
- **`SyncTarget`:** a console client (the deliverable) + `InMemoryHub` (tests / simulator §6).
- Lazy blob materialization + dedup by hash; **ref-conflict surfacing** (never silent-clobber
  `production`).
- **Console side:** the hub reconciler + the sync endpoints (now in scope).
- Wire BOTH station and the engine to sync via the same strategy.

**Milestone:** a model published on the engine appears on console after sync (and vice versa);
concurrent `production` moves surface as a conflict. Validated by the facade-only simulator (§6).
*Out: experiment/inference/dataset sync; P2P; retention + re-baseline hardening.*

### Later
Experiments + inference in the engine and their sync (append-only strategies); datasets +
annotation sync (the hard multi-writer case); resumable/prioritized blob transfer; retention +
re-baseline; station rebuilt on the engine.

## 6. Simulator / fake harness

Role: (a) conformance harness for the sync design, (b) **forcing function** for the one-facade
rule — it may perform **all domain operations through SDK types only**, and touch the engine
handle **only** for sync/lifecycle. If it ever needs an engine-specific *domain* method, that
is the signal a shadow facade is forming; fix it there, not in the harness.

Scenarios it must cover:

- **S1 — clean fast-forward.** B advances a ref; A pulls; auto-applies, no conflict.
- **S2 — concurrent ref conflict.** A and B both move `production` while partitioned →
  surfaces as `Conflict`, never silently resolved.
- **S3 — immutable convergence under partition.** Both publish new versions offline →
  both land as distinct hashes on reconnect, no loss, no conflict.
- **S4 — long-offline large divergence.** A offline for a "long" window accumulates many
  versions while the hub also moves → metadata converges; volume handled; refs correct.
- **S5 — partial materialization.** A has metadata for N versions, blobs for few; `load`
  triggers lazy fetch; offline `load` of an unmaterialized version fails gracefully.
- **S6 — re-baseline past horizon.** Merge base GC'd → falls back to re-baseline with local
  work preserved as a divergent lineage (rather than a silent merge).

## 7. Open questions (refinement surface)

- **Q1 — Engine crate location.** RESOLVED: the engine + sync live in `tracel-app` (the backend
  monorepo), reusing its `domain`/`application` crates (Path A, D2). Not in the SDK repo.
- **Q2 — Native store tech.** SQLite vs. structured files vs. an embedded KV (redb/sled).
  Affects portability to wasm later and how much we hand-roll.
- **Q3 — Relationship to `tracel-fleet`.** Fleet already has on-device sync machinery
  (WAL/outbox/shipper, model cache, `FleetClient`, its own `InferenceSink`). Do we reuse it,
  align with it, or supersede it? This is the biggest overlap risk and worth resolving early.
- **Q4 — Ref-conflict policy per ref class.** Which refs are human-resolve (`production`) vs.
  LWW-acceptable (convenience tags)? Product decision; the engine should expose it as policy.
- **Q5 — Human version numbering.** Hub-assigned display labels vs. per-lineage numbers vs.
  hashes-only. (A display number must never be identity.)
- **Q6 — Offline horizon.** How long is "long"? This sets blob + merge-base retention and the
  re-baseline threshold.
- **Q7 — `burn` dependency cleanup.** `tracel-experiment` pulls `burn` unconditionally
  (`pub mod integration` is not feature-gated). Feature-gate `integration`, or extract the
  ports/value-types into a burn-free leaf crate? Needed so the engine doesn't drag `burn`.
- **Q8 — Harness placement.** An `examples/` member vs. a dev crate under `crates/`.
- **Q9 — `SyncTarget` delta granularity.** Push/pull incremental deltas (cursor design) vs.
  full-state exchange for v1 simplicity.
- **Q10 — Single-handle ergonomics.** `engine.context()` returning SDK modules (recommended)
  vs. the app constructing `Context` itself.
- **Q11 — Per-domain sync protocol placement (bilateral contract).** LARGELY RESOLVED by Path A
  + engine-in-backend: each domain's sync strategy lives in its bounded-context crate (e.g.
  `model-registry`) and is shared **as code** by every composition root in the monorepo — the
  cloud reconciler (hub) and the engine (spoke) call the *same* merge. No translation contract to
  keep in lockstep. Remaining sub-question: nothing of this surfaces in the SDK (per D3) — confirm.
- **Q12 — wasm dependency surgery (now a Phase-0 task, per D11).** The reused `domain`/`application`
  crates must compile to `wasm32-unknown-unknown`: make `tokio`/`sqlx` optional in the bounded-context
  crates, remove the `sqlx::Type` leak in `inference-group`'s domain, and split
  `shared-infrastructure/local` so it stops dragging in `axum`/`utoipa`. Done per-context via the
  Phase-0 vertical, not monorepo-wide up front. **Open sub-question the spike answers:** is any
  context's application layer secretly native-coupled (tokio runtime / `std::fs` / blocking)? If one
  is genuinely hard, the fallback for *that* context is Path C — share only its domain, write a lean
  wasm-clean application in the engine. Risk note: the bounded contexts are mid-extraction, so this
  surgery lands on moving code.

## 8. Explicitly out of scope (this milestone)

- Cloud sync for domains other than model-registry (experiments/inference/datasets). The
  model-registry console sync IS Deliverable 2; the rest is later.
- Peer-to-peer sync.
- Dataset/annotation sync — the genuinely hard mutable-multi-writer case; deliberately later.
- Station rebuilt on the engine.

## 9. Deliverable 1 — Q12 surgery checklist (from the model-registry audit)

**Headline: the domain and application layers are wasm-clean.** `model-registry/src/domain/**` and
`application/**` are pure `async` orchestration over `#[async_trait]` ports — no tokio-runtime, `std::fs`,
threads, blocking, or `sqlx` derives. Path A holds for the registry with **no Path-C fallback needed.**
The blockers are the Cargo dependency graph + one error-enum leak + getrandom config.

> **Status — `shared-domain` foundation: DONE & verified (Path 1 gate).** Changes: `sqlx` made
> optional + the `BurnCentralError::Sqlx` variant gated behind a new `sqlx` feature; `tokio` made
> optional + folded into `test-utils`; the `sqlx` feature wired through `shared-infrastructure`'s
> `local`/`cloud` chains so the 547 native sites keep the variant untouched; wasm target deps added
> for getrandom (0.2 `js`, 0.3 `wasm_js`) + uuid (`js`). Verified: `shared-domain` compiles native
> (no sqlx/tokio), with `test-utils`, AND to `wasm32-unknown-unknown`; full native workspace + both
> composition roots (`burn-central-api` cloud, `burn-station-api` station) build clean. Files:
> `shared-domain/{Cargo.toml, src/errors.rs}`, `shared-infrastructure/Cargo.toml`.
> **Still needed for the wasm build to be reproducible without an env var:** a `.cargo/config.toml`
> with `[target.wasm32-unknown-unknown] rustflags = ['--cfg', 'getrandom_backend="wasm_js"']` (used
> via `RUSTFLAGS` during verification) — belongs with the engine crate.
>
> **Status — `model-registry` crate: DONE & verified.** Made `sqlx`/`tokio`/`shared-infrastructure`
> optional and folded into `local`/`cloud` (they live only in the gated `infrastructure/database/*`
> subtree). Verified: `model-registry` compiles to `wasm32-unknown-unknown` (domain + application,
> including `#[automock]` ports) on top of the wasm-ready `shared-domain`; native default,
> `test-utils`, and both composition roots build clean. File: `model-registry/Cargo.toml`.
> **Bucket A of §9 is proven end-to-end** — the reused domain/application cross to wasm; remaining
> Deliverable 1 work is the engine side (buckets C2/D: the `tracel-engine` crate + adapters).
>
> **Status — `tracel-engine` composition (Option A): DONE & verified.** New crate
> `backend/crates/tracel-engine` composes the reused `ModelAppService` with in-memory adapters
> (in-memory `ModelPersistence`, fixed single-tenant `ContextFetcher`, permissive `PermissionService`,
> no-op `EventBus`, blob/experiment stubs). Verified: native `cargo test` runs a `create_model` →
> `list_models` roundtrip through the real application layer; the lib compiles to
> `wasm32-unknown-unknown`. So the engine composition (Path A) works AND crosses to wasm with the
> reused domain logic. Next: durable storage (native SQLite / wasm OPFS) behind `ModelPersistence`,
> blob materialization, and the SDK facade wiring.

## 10. Model-registry surface (grounded in metabolic, the real consumer)

`metabolic` (the real "tracel-chat") is a Burn LLM inference app — egui, native + wasm (WebGPU),
**inference-only** (no training; "creating a model" = native offline quantization). It uses **no
tracel SDK today** (greenfield), and its browser build has **no weight cache and no persistence**
(OPFS/IndexedDB are TODO) — the gap the engine fills. Its loading is already `fetch(async) →
load_bytes(sync)` through a `Boot::Bytes`/`load_bytes` seam, and it already splits *base* models
(cached HF bytes) from *derived* models (its own SQLite catalog).

**Decisions from working against it:**

| # | Decision | Rationale |
|---|----------|-----------|
| M1 | **Registry = your labeled models only (option 1).** Downloaded public checkpoints are NOT registry entries; they're cache in a lower content-addressed byte store (shared infra, not the registry). | A model "in the registry" = a versioned model you own and sync. Public/unlabeled bytes have no registry semantics — they don't belong in the model-registry module. |
| M2 | **A version's artifacts = a bundle of arbitrarily-named files**, reusing `tracel-artifact`'s `Bundle`/`BundleSource`/`BundleSink`. The registry never interprets file names or contents. | No hardcoded `weights`/`tokenizer` shape (that's LLM-specific). metabolic writes/reads `weights.bpk`/`tokenizer.json` by name. |
| M3 | **App model semantics = opaque per-version metadata** (family/precision/quant/architecture). Stored and synced, never read by the registry. | metabolic's `ModelSpec` is app knowledge; the registry stays domain-agnostic. |
| M4 | **The load boundary is bytes, not a decoded model.** The registry yields artifact bytes (async fetch/materialize); the consumer owns decode/boot (sync). | metabolic's boot is its own. This kills the SDK's `load::<D: BundleDecode>` as the wasm load shape and matches metabolic's real `fetch(async) → load_bytes(sync)`. |
| M5 | **The supported-pretrained catalog stays app-side.** The registry does not curate public checkpoints or list "what you can download." | Curation of "what this app can run + where to fetch it" is app config, not a registry primitive. |

> **Revision (2026-07-17, user decision): M1 + M5 are superseded by §13** for the post-integration arc —
> catalog/base checkpoints become registry entries at download time and the registry becomes metabolic's
> only model store. M2/M3/M4 stand unchanged (and are what make the reversal cheap). Rationale, decisions
> (L1–L7), and phasing: §13.

**The surface (general; sync metadata core + async I/O edges):**
```rust
// your models — metadata, sync (in-memory-authoritative; egui reads in-frame)
registry.models() -> Vec<Model>
registry.model(name) -> Option<Model>
registry.versions(model) -> Vec<Version>          // version = manifest {path -> digest} + opaque metadata

// artifacts — async fetch, sync read
registry.fetch(model, version).await -> Bundle    // materialize files (cloud / OPFS cache)
bundle.open("weights.bpk") -> impl Read           // sync (tracel-artifact BundleSource)

// write + sync — async
registry.publish(model, version_spec).await       // content-addressed; dedups shared blobs
registry.sync(console).await
```
metabolic maps on directly: its derived/quantized models become `publish`ed versions (family/precision
as opaque metadata) and sync to console; loading a your-model is `fetch` → read files → its own sync
boot; public bases stay app-side (M5), optionally riding the shared byte store later. Note this makes
model-registry the first SDK module with **async** methods (`fetch`/`publish`/`sync`) alongside the sync
metadata reads — the honest consequence of M4.

> **Status — B (SDK facade wiring): metadata path DONE & verified.** On branches
> `tracel:feat/model-registry-provider` and `burn-central-app:feat/tracel-engine` (both uncommitted):
> - New SDK crate `tracel-model-registry` (types, `ModelRegistryProvider` port, `ModelRegistryModule`,
>   `Revision` handle), bundle-based artifacts, clean error. Compiles native + wasm.
> - `tracel-artifact` gated: reqwest behind an off-by-default `transfer` feature so the `bundle` traits
>   are wasm-buildable; `tracel-core` opts in. (Required, not cosmetic — blocking reqwest doesn't wasm-compile.)
> - `tracel-engine` implements `ModelRegistryProvider`: sync `models`/`model` run the reused async
>   `ModelAppService` over the in-memory store and drive it with `block_ready` (noop-waker `poll_once`,
>   no runtime, wasm-safe). `versions` empty until publish; `fetch`/`publish` stubbed until blobs.
>   `Engine::registry()` returns the SDK `ModelRegistryModule`.
> - Verified: `cargo test -p tracel-engine` lists models **through the SDK facade** via the sync bridge;
>   the whole chain (cross-repo path dep `tracel-app → tracel` + facade + `block_ready` + reused
>   domain/application) compiles to `wasm32-unknown-unknown`.
> So one-facade + Path-A + the sync-over-async bridge are proven end to end. Next: `fetch`/`publish`
> need the blob/OPFS storage step; then durable storage behind the in-memory store.

## 11. Storage architecture: metadata is domain, byte transfer is infra

**Principle.** Moving bytes is *not* a domain or an agnostic-application concern — it is per-context
infrastructure. The domain and application deal only in **metadata**: a version and its manifest of
logical files. How bytes physically reach and leave storage is chosen by the deployment context
(cloud vs. local) at the infra/API layer, and may be out-of-band. **The manifest is the only contract
that crosses contexts.** (This supersedes the "shared `BlobStore` port" idea from §10/prior turns —
there is no shared byte port; there is a shared manifest + per-context byte infra.)

```
Domain        ModelVersion = a manifest of LOGICAL files { rel_path, size, checksum }.
              No storage keys, no bucket, no bytes.
Application   - shared + agnostic: CreateModelVersion (allocate + record manifest), list / get.
              - context-specific use-cases, each over its OWN port (each reuses CreateModelVersion):
                  PublishViaStream (local)      → BlobStreamPort  (put / open, streaming)
                  PublishViaUploadPlan (cloud)  → UploadPlanPort  (presigned plans)
                  + symmetric Fetch use-cases.
Infra         adapters behind the ports:
                  BlobStreamPort → native fs / wasm OPFS streaming
                  UploadPlanPort → S3 presigned
API / root    thin: wires the context's use-case + adapter. No orchestration here.
```

The manifest is still the seam, but the byte orchestration lives in the application layer as
per-context use-cases — not in the API glue. Each is a first-class, testable use-case (mock the port).

On the local side, `BlobStreamPort::open` yields **`BlobHandle`s**, not a sync bundle — each exposes
`local_path() -> Option<&Path>` (Some on native → burn loads from the file, zero-copy) and
`read().await -> Bytes` (wasm → burn loads from bytes), because **burn loads differently per target**
(`Boot::Base`/file vs `Boot::Bytes`). This pins bytes-through-memory to the one unavoidable spot (wasm
load); native publish streams, native fetch is a zero-copy path, wasm publish streams.

**Redraw vs. today.**

Domain — remove the storage leaks:
- `ManifestFile { rel_path, size, checksum }` — drop `blob_key`. Today `BillableFileDescriptor` carries
  `blob_key: FileStorageKey`; a storage location belongs to infra.
- `ModelVersion` drops `bucket_id` — also a storage detail.
- `ModelVersion::create` builds the logical manifest only; it no longer computes `FileStorageKey`s.
- Infra derives the physical location deterministically from identity — `key(model, version, rel_path)`,
  bucket from project (cloud) or a fixed local root — a pure infra mapping, nothing persisted in the domain.

Application — shrink to metadata:
- Version creation becomes "allocate a version number + record its manifest" (+ validate). Takes logical
  file specs, returns the `ModelVersion`. **No `FileStorageClientProvider`, no presigned URLs, no bytes.**
- Today `UploadModelVersionAppService.upload_model` bakes presigning in (`model.upload_new_version(...,
  storage_provider) -> presigned_urls`). That moves out; the app no longer depends on the storage port.
- "Are a version's bytes present and valid" is a completeness **query** the app can require, answered by
  infra — not something the app orchestrates.

Application — the context-specific publish/fetch use-cases (each reuses `CreateModelVersion`):
- **`PublishViaUploadPlan`** (cloud) over `UploadPlanPort`: create-version → produce a presigned upload
  plan for the manifest → [client uploads out of band] → verify + mark complete. Bytes bypass the server.
- **`PublishViaStream`** (local) over `BlobStreamPort`: create-version → stream each file's reader to
  `key(...)`, hashing as it goes → the manifest's checksums come from the stream. No URLs, no proxy.

The byte *mechanics* (S3 presigning, fs/OPFS streaming) are the port **adapters** in infra; the
composition root just wires the right use-case + adapter.

**Flows (the ordering that falls out):**
- `publish`: allocate version → derive keys → stream each bundle file to its key while hashing (size +
  checksum) → record the version with the resulting logical manifest. One stream per file; nothing fully
  buffered.
- `fetch`: load the version → for each manifest file, open a stream from `key(model, version, rel_path)`
  → hand back a streaming bundle; verify checksum on read if desired.

**Consequences / execution notes:**
- Removing `blob_key`/`bucket_id` from the domain touches `ModelVersion`, `BillableFileDescriptor` →
  `ManifestFile`, the DTOs, and fixtures. Real but mechanical.
- `FileStorageOps` splits into two proper ports: `UploadPlanPort` (delegated/presigned; cloud adapter)
  and `BlobStreamPort` (direct streaming; native-fs and OPFS adapters). The domain stops referencing
  storage entirely.
- Reuse: the domain + `CreateModelVersion` (metadata) are shared; the context publish/fetch use-cases
  and their port adapters are per-context — correct, because the byte strategy genuinely differs.

## 12. Resume here — current state & next steps

**Committed** (feature branches):
- `tracel:feat/model-registry-provider` — `tracel-model-registry` crate (SDK contract) + `tracel-artifact`
  reshape. **`fetch` returns a `BundleSource` bundle** (`Artifacts`, the *same* type as the publish input —
  symmetric); `tracel-artifact::BundleSource` gained `local_path() -> Option<&Path>` (default `None`;
  `FsBundle` overrides) so consumers can load lazily / zero-copy from a real file (feeds burn-pack's seekable
  `Reader::from_file`) instead of streaming. The bespoke `Fetched`/`ArtifactFile` (eager `Vec<u8>`) were
  **deleted** — async is a materialization concern, not a read concern; reads stay sync. Also
  `ModelRegistryModule::create_model`, `Revision::latest()`, and `Version.metadata`. See the streaming north
  star (seek-based lazy load to cubecl; burn branch `feat/burnpack-streaming`) — deferred.
- `burn-central-app:feat/tracel-engine` — wasm surgery (shared-domain/model-registry) + the `tracel-engine`
  composition root, with the hexagonal use-case refactor (below) + the wasm `Engine::open_opfs` constructor
  over an OPFS `BlobStreamPort` adapter.

**Architecture correction — supersedes the old "ENGINE-ONLY, inline the orchestration" framing.** Publish
and fetch are NOT orchestrated in the engine. `tracel-engine` is an API adapter + composition root only; the
orchestration lives in `model-registry`'s application layer as focused use-case services:
- `domain/model/blob_stream.rs` — the `BlobStreamPort` driven port (+ `BlobHandle`/`BlobStat`/`BlobLocation`),
  keyed by **logical identity** `(model_id, version, rel_path)`; the adapter derives the physical path. This
  is the LOCAL streaming storage model, distinct from the cloud-delegated presign model
  (`FileStorageClientProvider`).
- `application/services/publish_model_version.rs` — `PublishModelVersionAppService` over `ModelPersistence`
  + `BlobStreamPort`; declares an SDK-free `ArtifactSource` input port. Streams each file, builds a **logical**
  manifest, creates the version via the new domain constructor `ModelVersion::create_local`, upserts.
- `application/services/fetch_model_version.rs` — `FetchModelVersionAppService`; resolves the version, opens
  a `BlobHandle` per file, returns a `MaterializedVersion`.
- `tracel-engine`: `storage.rs` `FsBlobStore` *implements* `BlobStreamPort`; `registry.rs` is thin delegation
  (`models`/`model`/`versions` → `ModelAppService` query methods via `block_ready`; `publish`/`fetch` → the
  use-cases; `fetch` builds a `BundleSource` from the materialized blob handles — a lazy path-backed
  `PathBundle` on native, an `InMemoryBundleReader` on wasm); `lib.rs` `Engine::open` wires the adapters +
  three services.

**Storage-model decision (this pass): clean use-case, DEFER the §11 domain field removal.** `create_local`
builds a logical manifest and sets the still-present `bucket_id`/`blob_key` to honest local placeholders (no
bucket; key = `rel_path`), never a cloud location. The cloud presign path is untouched. New local use-cases
never touch `FileStorageClientProvider`.

**Verified (native + wasm):**
- `model-registry`: `cargo test -p model-registry --features test-utils,local,cloud` → 83 pass (use-case tests
  + sqlite AND postgres DB round-trips, incl. metadata). wasm builds.
- `tracel-engine`: `cargo test -p tracel-engine` → 6 pass (native: create/list/publish+metadata/fetch/latest
  revision/missing-version, FS blob round-trip). wasm builds (getrandom backend via `backend/.cargo/config.toml`).
- `tracel-engine`: `cargo test -p tracel-engine` → 7 pass (native; adds a durability *survive-a-reopen* test).
- `tracel-engine` wasm runtime: `wasm-pack test --headless --chrome` → the OPFS create→publish(+metadata)→drop→
  **reopen**→read round-trip through the SDK facade passes in a real (headless) browser (durability proven).
- cloud (`burn-central-api`) + station (`burn-station-api`) build clean; `cargo clippy` clean on the touched crates.

**Done since (registry completeness — solid + SDK-integrated before sync):**
- **Opaque per-version metadata (M3)** round-trips end to end: a `metadata: Value` field on the domain
  `ModelVersion` (set by `create_local`; the cloud `create` defaults `Null`), through the DTO, the streaming
  publish use-case, and the SDK `Version`. Persisted **engine + cloud**: new `metadata` column on both DB
  adapters (postgres migration `0044`, sqlite/station `0012`). No DB migration was needed for the engine
  (in-memory store), but the cloud/station path now stores it too.
- **Model creation on the SDK facade:** `ModelRegistryModule::create_model(name, description)` (new provider
  method; the engine delegates to `ModelAppService::create_model`). The engine's bypass `create_model`/
  `list_models` are removed — `Engine::registry()` is the single entry point.
- **`Model.latest_version`** is populated (via `get_latest_model_version` in the app-service DTO assembly).
- **`Revision::latest()`** resolves in `fetch`/lookup (engine resolves it through `get_project_model`, honoring
  the layering — no direct port access from the API adapter).

The model registry is now feature-complete for Deliverable 1 (create/list/versions/publish/fetch + metadata +
latest), native and wasm.

**Done since (durable metadata — the in-memory-authoritative design, §9 realized):** the engine's metadata now
**survives a restart**, on both targets. Design as decided:
- **In-memory stays authoritative** (the SDK reads are *sync* via `block_ready`, so they must never suspend on
  I/O). Durability sits *behind* memory: hydrate-at-open + write-through on mutation; the durable side is only
  written and read-once-at-hydrate, never queried.
- `DurableModelStore` (a `ModelPersistence` impl) = `InMemoryModelStore` (reads) + a `dyn CatalogStore` seam
  (the durable medium: `FsCatalogStore` native / `OpfsCatalogStore` wasm — same role `BlobStreamPort` plays for
  blobs). Chose a **JSON catalog snapshot** over SQLite because a queryable DB is wasted when memory is
  authoritative; whole-catalog write per mutation (per-record is a later optimization). Serialize domain
  `Model`/`ModelVersion` **directly** (they already derive serde for the DB manifest + will for the sync wire),
  so no mapping DTOs; needed only one shared-domain fix (`ExperimentId` gained `Deserialize`).
- **`Engine::open`/`open_opfs` are now `async` and return `Result`** — they hydrate from the catalog (and a
  corrupt catalog surfaces as an error).

Earlier milestones also landed: the wasm engine (`Engine::open_opfs` + the OPFS `BlobStreamPort` adapter,
`send_wrapper` for the `Send`/`!Send` JS bridge) and the hexagonal use-case refactor.

**In progress — Metabolic integration (prove-the-API-shape pass, BEFORE sync).** Integrating the registry
into `metabolic` (the real "tracel-chat") to validate the publish/fetch/metadata/load surface against the
real consumer before building sync on top of it. Decision: **full replacement** — the registry becomes the
single source of truth for metabolic's *derived* models on both targets, retiring the native SQLite
`derived_models` catalog and the browser `manifest.json` runtime catalog (base/pretrained stays app-side, M5).
Approved plan: `~/.claude/plans/zippy-crafting-puzzle.md`. Phasing: 0 compile-spike / 1 `metabolic-registry`
adapter crate / 2 `delete` verb / 3 native cutover / 4 browser cutover + OPFS import / 5 cleanup.
- **Phase 0 DONE & verified:** new `metabolic/crates/metabolic-registry` (path-deps `tracel-engine` +
  `tracel-model-registry` + `tracel-artifact`) compiles native AND wasm32 alongside metabolic's burn/cubecl
  pins. Dependency surface is clean (getrandom flag already in metabolic's `.cargo/config.toml`; `tempfile`
  builds on wasm; versions align). The load seam maps 1:1: metabolic's `PortedWeights::{File,Bytes}` ↔
  `BundleSource::{local_path,open}` (native mmap / wasm bytes).
- **Phase 2 DONE & verified:** the registry now has a **`delete(name)` verb** (the gap full replacement
  forces). Added on the SDK facade (`tracel-model-registry` `ModelRegistryProvider`/`ModelRegistryModule`),
  a **backend-agnostic** `DeleteModelAppService` (permission → per-version artifact removal → metadata delete)
  over `ModelPersistence::delete_model` (all 4 stores + automock) **and a new agnostic `ArtifactRemoval` port**
  (`remove(&BlobLocation)`), plus the engine wiring. **Delete is deliberately agnostic, not a local-streaming
  use-case:** removal has no API-bound I/O (unlike publish/fetch's streaming-vs-presign), so `BlobStreamPort`
  stays transfer-only (put/open) and both the local blob stores and (later) the cloud `delete_object`
  implement `ArtifactRemoval` — one delete serves every backend, no duplicate. Engine-local wiring only this
  pass (no cloud/station HTTP route, no cloud blob cleanup); stays clear of the presigned path. When
  content-addressing + dedup land, eager artifact removal moves to GC and the metadata delete stays the stable
  agnostic core. Verified: `model-registry` 85 tests, `tracel-engine` 8 native + 2 wasm (headless-Chrome OPFS
  delete), cloud + station + tracel workspaces build clean.
- **Phase 1 DONE & verified:** `metabolic-registry` adapter fleshed out (burn-free): `RegistryMeta`↔`Value`
  mapping (manifest-compatible tokens), a `Registry` handle (open native / open_opfs wasm) exposing
  metabolic-shaped ops (list/meta sync; publish auto-creating the model, fetch, delete async), bundle-type
  re-exports, native `pollster` `block_on`. The `BundleSource`→`PortedWeights` conversion stays provider-side
  so the adapter never pulls burn. Surfaced one API-shape finding: `versions()` errors `ModelNotFound` for an
  absent model, so the adapter smooths lookups to `None`. Verified: 4 native + 1 wasm/OPFS (headless Chrome).
- **Phase 3 implemented (uncommitted in `metabolic`), design-reviewed & corrected:** native `embedded.rs`
  cutover — quantize→publish (staged dir → `FsBundle`, staging copy removed after), load via the registry,
  list/meta/delete via the registry, adopt-on-init migration of legacy store rows (idempotent; leaves the
  old rows + cached copies in place — retiring the table is Phase 5). The load seam is **per-file, not
  per-directory**: a first cut reverse-engineered a *directory* from `local_path("model.bpk").parent()` and
  re-joined file names — that bakes the `FsBlobStore` layout (one adapter's internal detail) into the
  consumer and breaks under content addressing (D4/D6), where a version's files won't be co-located. It was
  reshaped: `metabolic-inference` gained `DerivedArtifacts { weights, tokenizer }` (each file's own path);
  the pool's resolver returns it, `Boot::DerivedFiles` carries it, `load_derived_files(weights, tokenizer)`
  replaced `load_derived_dir(dir)`, and `embedded.rs` resolves each file individually through the bundle.
  The id-keyed `Boot::Derived` variant was then collapsed away — it duplicated `DerivedFiles` with the
  path resolution merely deferred. `Model::load_derived(id)` / `AsyncModel::load_derived` remain as
  native-gated conveniences that resolve the cache's files and delegate; the cache layout itself is owned
  by `metabolic-models` (`derived_cache_files(id)`), so the file-name convention lives in one place. One
  derived boot path; the registry and the cache are just two resolvers feeding it.
  Contract docs hardened to match: `BundleSource::local_path` now states files are located individually
  (co-location is NOT part of the contract; path lifetime is source-defined), and the engine's `PathBundle`
  documents its paths as stable store locations valid until the version is deleted (what lets the consumer
  drop the bundle and mmap later). The artifact-set names live once, as `metabolic-registry`
  `WEIGHTS_FILE`/`TOKENIZER_FILE`. Verified: `metabolic-registry` 4 native tests, `metabolic-backend`
  native (`--features cpu`) + wasm (`wasm-embedded`) checks clean, `tracel-engine` 8 tests.
- **Phase 3 runtime-verified** (live `metabolic serve` on the SSE/REST protocol, isolated
  `METABOLIC_CACHE`/`XDG_DATA_HOME` scratch with real cached checkpoints + a real `chat.db`): adopt-on-init
  published both legacy derived models (2.4GB copied+hashed in ~6s, acceptable); reopen hydrates from the
  catalog and the migration skips already-adopted models (blob mtimes untouched, versions stay 1); the
  picker lists registry-backed entries (manifest sizes); delete drops blobs+catalog+store row, survives
  restart, silently no-ops on an unknown id, and a deleted name re-adopts as a fresh model. **Two real bugs
  found and fixed:** (1) the app id `derived/<slug>` was used as the registry model name — the domain's
  name validation rejects `/`, so every adopt and create failed at runtime; the registry now knows models
  by the bare slug (`registry_name`/`derived_app_id` map at the embedded.rs boundary). (2) engine artifact
  removal left empty `models/<id>/versions/<v>` directory skeletons; `FsBlobStore::remove` now prunes
  emptied layout dirs up to the store root. **Not verified (blocked, pre-existing):** completing a weight
  load, generation, and quantize-create — every backend (cpu/vulkan/cuda) dies loading ANY model (base
  included) with a cubecl "can't allocate buffer of size: 44040192" panic on current main; the registry
  fetch → per-file `local_path` → mmap record read demonstrably ran (the load died in device upload,
  after the changed seam). Suspect the capability-metrics/Runtime merge's pool layout; tracked separately.
- **Phase 3 review — flagged follow-ups (not blocking):** (1) adopt-on-init copies every legacy model's
  bytes synchronously inside worker init (GBs before the UI answers, no progress events) — background it
  or wait for ingest-by-move; (2) create pays a double disk write (quantize→staged copy, publish
  stream-copies into the store) — a local **ingest-by-rename/hard-link** fast path on the blob store is
  the SDK gap to close (stays compatible with content addressing: hash, then link into place); (3) the
  facade has no single-version getter (`version(name, revision)`) — the adapter lists all versions to read
  the latest's metadata/manifest; symmetric with `fetch` and worth adding when convenient; (4) publishing
  a new version doesn't evict a warm pool entry for the same id — stale weights serve until eviction
  (behavior parity with the old overwrite model, more visible with versioning).
- **Phase 4 implemented & compile-verified (committed in `metabolic`; runtime verification deferred at
  the user's request, alongside the re-check of the cubecl load blocker).** The browser provider's
  runtime catalog is the OPFS registry (`Registry::open_opfs("metabolic-registry")` at init); the
  weights manifest is demoted to a discovery source. The model list is the union — manifest order with
  `downloaded` now a real cached/not distinction, plus registry extras the manifest no longer offers.
  First load of a manifest model: download → `publish` into OPFS → verify the manifest's sha256 against
  the store's own digests (the adapter's `publish` now returns them as `Published`; a mismatch rolls the
  import back) → fetch back from the registry, so one code path serves every load and what boots is what
  the store holds. Page reloads list + load from OPFS with no re-download. Both delete verbs
  (`delete_model`/`delete_model_weights`) drop the OPFS copy; a still-published model reverts to
  not-downloaded. No-OPFS degrades to the old download-per-session path. Verified: `metabolic-registry`
  4 native + 1 wasm/OPFS (headless Chrome, digest assert added), `metabolic-backend` cpu + wasm32
  clippy-clean, `metabolic-ui --no-default-features --features local-compute` (the real trunk/ui-heavy
  build) checks clean on wasm32.
- **Phase 4 API-shape finding:** on wasm, `fetch` materializes the bundle in memory
  (`InMemoryBundleReader`) and the only read path through the `BundleSource` trait object is
  `open()`+`read_to_end` — a **transient second copy** of the weights at every load (bundle's buffer +
  the caller's Vec) that the facade gives no way to avoid (no take/into-bytes escape hatch, and
  `local_path` is native-only). Acceptable at the current model tier; the seekable-source north star or
  a consuming accessor on the bundle removes it. Record for the SDK pass.
- **Phase 5 done (committed in `metabolic`) — the cleanup.** The `derived_models` table is write-retired
  to a legacy bridge: nothing writes it (`insert_derived_model` removed); it is read once by the
  adopt-on-init migration, and rows drop on model delete (kept deliberately — the settings-row cleanup
  and the anti-resurrection tombstone: a legacy row + cached copy left in place would re-adopt a deleted
  model at next start). Quantize staging moved out of the checkpoint cache into a transient
  `cache_root()/staging/<slug>` beside the registry (same filesystem — ready for L6 ingest-by-move),
  swept at init; `create_quantized`/`quantize_from_hub` take an explicit output dir now. The id-keyed
  derived-cache load chain is gone: `Model::load_derived`, `AsyncModel::load_derived`,
  `TransformerConfig::load_derived`, `DerivedArtifacts::in_cache`, `derived_cache_files` → replaced by
  the explicit-path forms (`load_derived_at`/`load_derived_files`, `DerivedArtifacts::staged_in(dir)`,
  `staged_files(dir)`); the dev tests/examples pass `cache_dir(id)` explicitly as their persistent
  workspace. Stale `WASM_SUPPORT.md` doc refs repointed at the contributor book. Verified: backend lib
  28 tests pass (incl. the reworked legacy-row store test), models tests + inference tests/examples
  compile (cpu), both wasm leaves (`metabolic-backend` wasm-embedded, `metabolic-ui` local-compute)
  check clean, clippy clean on touched files. Pre-existing failures noted, untouched: bare-features
  `metabolic-models` check dies in `metabolic-extension`; the `metabolic-benchmark-bench` lib fails on
  burnbench API drift (its two mechanical example fixes are compile-blocked behind that).
- **Runtime verification done (isolated env, RTX 4060/CUDA + headless Chromium) — the metabolic
  integration arc (Phases 0–5) is COMPLETE.** Native: the old cubecl "can't allocate buffer" load
  blocker is resolved by main's KV-cache fixes — base load + generation pass, and the full registry
  lifecycle passes (quantize → `staging/` → publish → staging removed → registry-backed listing →
  load 1.75s → generate → delete drops blobs+catalog and survives restart). The legacy adopt-on-init
  migration also validated on real data (a concurrent real-cache run adopted 2 derived models cleanly).
  Browser: publish/serve, first-load download → OPFS import ("cached … in the model registry (OPFS)"),
  reload lists as downloaded and loads from OPFS with NO model.bpk network fetch, delete reverts to
  not-downloaded and persists. Browser *generation* NOT VERIFIED — headless Chromium only exposes
  SwiftShader WebGPU (no shader-f16), and the boot path correctly surfaced the typed
  "cannot compute in F16" error instead of crashing; expected to pass on real hardware WebGPU.
  No regressions attributable to the cutover commits. Environmental notes: non-fatal cubecl 512MB
  autotune-thread panic on the 8GB card (pre-existing); trunk currently doesn't build on this machine
  (`libdeflate-sys` vs GCC 16.1 — wasm bundle was built by hand for the test); headless OPFS writes
  slow (~1.5 MB/s, likely a headless artifact — glance at real-hardware import speed later).
  Next: the §13 arc (registry as metabolic's single model store), Phases 6–9.

**Refined design ready (2026-07-17) — START HERE for Phases 6–9.** The ingest-by-move + origin/availability +
identity exploration is written up as refined decisions at the end of §13 ("Refined design — …"), subsections
A (ingest, reverted), B (origin/availability, landed), **C (identity model — the Phase 8/9 substrate)**. It pins
the port reshape, the checksum/semantics/pull forks, and — new — the registry-canonical identity decision (C1
explicit `kind`, C2 one resolver, C3 list-from-registry) that Phases 8–9 build against. **Phase 6's
availability/origin scaffolding landed; ingest-by-move was built then reverted (CAS pass). Phase 7 base ingestion
is done + test-verified. Next: Phase 8 (quantize-as-fork) on the C1/C2 substrate — see §13.C.**

**Phase 6 — availability/origin scaffolding DONE (committed); ingest-by-move BUILT THEN REVERTED (user
decision 2026-07-17). Per the §13 refined design:**
- **Ingest-by-move — reverted, deferred to the CAS / base-ingestion pass.** It was implemented (a
  `BlobStreamPort::ingest` consume verb, an `ArtifactSource::local_path` opt-in, `FsBlobStore` hash-on-ingest
  + `rename`) and then pulled: publish streams every file through `put` again, and the domain/application
  ports are filesystem-free (no `ingest`, no publish-input `local_path`, no `&Path`). Rationale: the second
  write it saves only matters on multi-GB bases (Phase 7+), and doing it *correctly* is entangled with
  content-addressing (D4) — shared-blob GC/refcounting, the §11 logical-domain cleanup, and §13's
  base-vs-derived identity (imported bases are origin-addressed, NOT ported-bpk-hash-addressed, since porting
  isn't bit-reproducible). Until CAS lands `ingest` just shadows `put` and leaks the filesystem into the
  domain. §13.A stays as the recipe for reintroducing it (as pure-rename + source-carried digest, option b)
  when CAS / base-ingestion lands. Do NOT re-add it as an "obvious optimization" before then.
- **Availability (kept):** typed SDK `ModelRegistryError::ArtifactsUnavailable{name,version}` (distinct from
  `VersionNotFound`/`Backend`); `BlobStreamPort::exists` (default = open-probe; FS override `is_file`;
  the OPFS lookup classifies `DomException` `NotFoundError` → typed BLOB not-found, so absent stays distinct
  from a real fault); `FetchModelVersionAppService::availability` (computed local state — probed, never
  stored; shares a `resolve_version` prologue with `execute`); SDK `Availability{Present,Absent}` +
  `availability()` on provider/module; the engine maps an absent blob on fetch to `ArtifactsUnavailable`. The
  `fetch` docs now state the decided semantics: local-only materialization, pull is an explicit engine
  op (Deliverable 2) — the old "downloading if needed" wording is gone.
- **Origin (kept):** `RegistryMeta.origin: Option<OriginMeta{hub_repo, revision?}>` in `metabolic-registry`
  (opaque to the engine, honors M3/L5; backward-compat decode test). Populated at Phase 7 base
  ingestion; quantized derivatives stay origin-less (byte-synced, L4); browser manifest imports too
  (host-served, no hub recipe).
- Verified after the revert: `model-registry` 87 tests, `tracel-engine` 9 native + 2 wasm (headless OPFS,
  incl. availability), clippy clean on touched crates (native + wasm), engine wasm build clean. The SDK-side
  availability/origin (tracel + metabolic) is unchanged from Phase 6. **Next: Phase 7 (base ingestion).**

**Phase 7 implemented & test-verified (committed in `metabolic`; runtime verification pending) — base
ingestion: chat base checkpoints are registry models.**
- **Ingestion is lazy, on first load — a deliberate deviation from the §13 bullet's "adopt-on-init"**:
  with ingest-by-move reverted, publish stream-copies, and adopting a user's multi-GB base cache at
  worker init would stall startup with no progress UI (the exact pain follow-up (1) flagged). Instead
  the pool's resolver ingests on first need: a complete legacy cache copy publishes from the cache
  (left in place — it is the user's); a never-downloaded base downloads + ports into `staging/` (new
  `TransformerConfig::port_into` + `Model::port_base`, mirroring `create_quantized`'s stream/pool
  hygiene) then publishes and clears staging. Every later load fetches per-file paths from the
  registry (same mmap seam as derived).
- **Load seam:** `Boot::BaseFiles { repo, artifacts }` (config = `base_model_config`, file load shared
  with `DerivedFiles` — it is config-agnostic); `Model::load_base_at`; the pool resolves artifacts for
  BOTH key kinds now (`Model::load`/`load_hub` remain for dev tools only).
- **Naming:** registry name = repo with `/`→`--` (mirrors the cache's `models--` convention; legal
  chars, and always contains `--`, which a derived slug never does — disjoint namespaces). Nothing
  parses the name back: the exact repo travels in `origin.hub_repo`, which is also the app-side
  discriminator when listing (origin `Some` = ingested base, `None` = derived).
- **Surface:** `emit_models` chat entries read downloaded/size from the registry first (legacy cache
  still counts as downloaded pre-adoption); "Remove from disk" deletes the registry copy AND the
  legacy cache copy; STT/T2I stay cache-probed (L7). Base meta records family/source/ported
  precision/context/display name + `origin{hub_repo}` (the first origin producer).
- Verified: `metabolic-backend` 32 tests (+4: naming disjointness, base meta origin, lazy adopt
  idempotent + legacy-left-in-place, port-into-staging + staging cleared), inference tests/examples
  compile, both wasm leaves + server/cli build, clippy clean on touched files. Runtime pass (real
  download → ingest → load → generate, delete flows) still to run.
- **Identity-cohesion review (2026-07-17) → decision §13.C.** Phase 7's scheme works but the registry is
  not yet the identity source of truth: the base list is catalog-driven (an off-catalog ingested base
  would not list), the app-id↔registry-name map is a lossy per-kind projection at the boundary, and
  base-vs-derived is discriminated two different ways (name-lookup in `model_key`, `origin.is_some()` in
  `emit_models`). No active bug pre-sync (the app owns every write). Fix = registry-canonical identity
  (§13.C): explicit `kind` in `RegistryMeta` (C1), one `resolve`/`app_id_of` (C2), list-from-registry with
  catalog-as-discovery (C3). **Land C1+C2 as the Phase 8 substrate before fork.**

**Phase 8 (quantize-as-fork) — backend DONE & test-verified (committed in `metabolic`).** Scope note: "fork =
a new model, not a new version" was *already* the behavior (each distinct name mints `derived/<slug>`; the
quantizer already streams shards from the hub, L4), so the new backend piece is the **`derived_from` lineage**:
`RegistryMeta` gained an optional `derived_from{model, version}` pointing at the parent's **registry name**
(§13.C — resolvable inside the registry namespace; for a base it flattens the hub repo, so origin is recoverable
without an out-of-band string). Set for every derivative (forks + adopted legacy rows) in `to_registry_meta`;
a base and a browser manifest import leave it `None`. Version left unresolved (forking needs no ingested base; a
base is usually one version). Same additive-scaffolding pattern as `origin` — opaque `Value` to the engine,
metabolic-only. Verified: `metabolic-registry` 6 native + 1 wasm/OPFS, `metabolic-backend` 34 native (fork
records lineage at the base registry name), both wasm leaves + clippy clean.
- **Identity substrate landed and went deeper than C1/C2 → C7 full single-identity re-key** (see §13.C): derived
  id == registry name (prefix dropped), base resolved by `origin.hub_repo` query (naming write-time-only), store
  migration strips the retired prefix. The registry is now the genuine SoT on native; `derived_from` references the
  portable parent app-id. Browser unaffected (1:1). Fixes C6-for-bases.
- **Remaining before Phase 8 is "done-done":** the UI half (Phase 9 territory) — launch a fork from a Library
  card, resolving the parent's `origin.hub_repo` to feed the quantizer; and (optional) resolve `derived_from.version`
  to the base's ingested version at fork time. Plus the still-pending Phase 7 runtime pass (real download → ingest
  → load → generate), foldable into a Phase 9 runtime pass.

**Phase 9 (Library/Catalog split, C3) — DONE & build-verified (committed in `metabolic`); runtime pass in
progress.** The backend model surface was reshaped so the frontend never infers taxonomy: `BackendEvent::Models`
now carries two typed lists — `LibraryModel` (registry SoT: explicit `kind`, quant, `base` lineage, `origin`,
size, `LibraryState` stubbed `Local`, active, starred) and `CatalogModel` (discovery: curated `REGISTRY` chat +
speech/image native / weights manifest browser, each marked `downloaded`/`in_library` by an origin/id join).
`emit_models` is registry-first on both providers; the flat `ModelEntry` + every `base_repo.is_some()`/`bits`/
`modality` heuristic are gone at the source (C3 realized; catalog demoted to discovery, L1). The egui UI was
rewired to a Library|Catalog tab split — Library sectioned Preferred/Base/Derived, Catalog by modality — with
actions gated on explicit `kind` (Quantize on a Library Base chat, Delete on a Library Derived, Remove-from-disk
on a base or downloaded catalog entry); the fork seeds from a Library Base card; the agent pickers draw from a
library∪catalog chat union. Verified: `metabolic-backend` 36 tests, both `metabolic-ui` leaves (cpu + the real
`local-compute` wasm build) + server/cli build, clippy clean on touched files.

**Runtime pass (RTX 4060/CUDA, isolated scratch) — Phases 8, 9, identity/migration PASS; found + fixed a base-load
regression.** Verified working: quantize-fork (derived model created with `derived_from` lineage, staging cleaned,
loads + generates coherent text); the §13.C single-identity (derived id is a bare slug in library + agent binding)
and the chat.db prefix-strip migration (seeded `derived/<slug>` agent binding + settings row → stripped on
startup); the two-list SSE `Models` event shape (library/catalog with in_library/downloaded flips on
ingest/delete); delete flows survive restart. **Found: base checkpoints could not be loaded** — `save_ported`
writes projections **untransposed (Row)** but the base load path used `WeightsUse::Dense` (Col), so every base
load failed the record shape check ("expected [2048,1024] but record [1024,2048]"). Long-latent (warm base
reload + the browser dense-bytes path were always affected; the earlier cubecl blocker masked it); Phase 7 made
it unconditional by routing every base load through the file. **Fixed** (`46b7c2a`): split the conflated dense
op like the existing `QuantizedCreate`(Col)/`QuantizedLoad`(Row) pair — new `WeightsUse::DenseLoad`(Row) +
`base_load_config`, routed `Boot::BaseFiles` and the dense `Boot::Bytes` (browser) through it; porting keeps
Col. **GPU re-verified CORRECT (coherent generation):** base `Qwen/Qwen3-0.6B` and a freshly-ported
`Llama-3.2-1B` both load via `Boot::BaseFiles`→Row and generate coherent, correct text ("Paris", "4") — not
garbage; derived still generates correctly (regression guard). The true origin is sharper than "Phase 7": the
Jul-16 "Fix model-loading" commit changed `save_ported` to write Row and updated the *quantized* load to Row but
left the *dense base* load at Col — so base loading broke Jul-16; Phase 7 (Jul-17) made it unconditional and this
fix completes it.
- **Migration caveat (surfaced by the runtime pass):** base registry blobs **ported before the Jul-16
  save-format change are Col** and will fail the now-correct Row load; a one-time re-port (delete + reload, which
  re-ports from origin) fixes each. In the current real dev registry this affects only the pre-Jul-16
  `Llama-3.2-1B` blob (Qwen's Jul-17 blob is already Row). Optional hardening: auto-re-port a base from its
  `origin` on a load shape-mismatch (a natural use of the origin recipe) — deferred unless wanted.

**The metabolic integration arc (Phases 0–9) is COMPLETE and runtime-verified end to end.** Next frontier:
Deliverable 2 (sync).

**Later — Deliverable 2 (sync). Keep the use-case layering; DEFER the cloud restructure + §11 domain cleanup:**
- **Model-registry `DomainSync` strategy + a local `SyncTarget`** — compute the version's content identity over
  the **logical** fields only (`rel_path`+size+checksum); exclude `blob_key`/`bucket_id`. Domain `Model`/
  `ModelVersion` now serialize, which the sync wire can reuse.

**Later, one combined pass:** the §11 domain cleanup (logical manifest, drop `blob_key`/`bucket_id`, presign →
cloud `UploadPlanPort`) **and** the cloud hub side of sync.

**Durable store — known follow-ups (not blocking):** the snapshot is rewritten whole on every mutation
(fine at local scale; per-record incremental is the optimization); native writes are atomic via temp+rename,
wasm OPFS relies on `createWritable` close semantics.

**Constraints a fresh session must honor:**
- Use-case orchestration lives in the bounded-context application layer; the engine is API adapter +
  composition root only — never reach through persistence/blob ports to orchestrate from the engine.
- Do not touch the cloud presigned path; local use-cases use `BlobStreamPort`, not `FileStorageClientProvider`.
- Metadata reads are sync via `block_ready` (valid only while the in-memory store never suspends); byte and
  network ops are async.
- `BlobStreamPort` is direct-stream (engine/station), keyed by logical identity; cloud stays presigned. No
  universal byte port (§11).
- `fetch` returns a `tracel-artifact` `BundleSource` (the `Artifacts` type, symmetric with publish input).
  Reads stay sync (`open`); `local_path()` exposes a real file for lazy/seekable loading (burn-pack
  `Reader::from_file`) — native is path-backed and lazy, wasm materializes to memory. Do not reintroduce an
  eager `Fetched`/`ArtifactFile` shape.
- wasm needs the getrandom `RUSTFLAG` (belongs in a `.cargo/config.toml` with the engine eventually).
- Honor the recorded decisions: D1–D11 (§2), M1–M5 (§10; M1/M5 revised by §13 for the post-integration
  arc), the §11 storage principle. Path A (reuse domain/application), engine lives in `tracel-app`
  (backend), SDK stays untainted.

**Cross-repo dep:** `tracel-engine` → `../../../../tracel/crates/tracel-model-registry` (path); test
bundles use `tracel-artifact` (dev-dep).

**A. Compiles as-is (no change):** domain + application logic; all ports (`ModelPersistence`,
`ContextFetcher`, `ExperimentPersistence`, `PermissionService`, `FileStorageOps` /
`FileStorageClientProvider`, `EventBus`); `shared-domain` value types.

**B. Cargo cfg-gates (dependency surgery, no logic change):**
1. `shared-domain/Cargo.toml` — `tokio` and `sqlx` are unconditional via `workspace = true`
   (`tokio=["full"]` → `mio`; `sqlx=["runtime-tokio-rustls"]` → `ring`). Make `sqlx` optional (pairs with
   C1); on wasm slim `tokio` to `["sync","macros"]` via a `[target.'cfg(target_arch="wasm32")']` override.
2. `model-registry/Cargo.toml` — make `shared-infrastructure` and `sqlx` optional, folded into the
   existing `local`/`cloud` features (used ONLY in the gated `infrastructure/database/*` subtree; with
   neither feature the infra body is empty). Slim `tokio`.
3. getrandom — add a wasm target dep enabling `getrandom` 0.2 **`js`** (aes-gcm `OsRng` in
   `shared-domain/src/encryption.rs`) and 0.3 **`wasm_js`** (uuid v4), plus
   `--cfg getrandom_backend="wasm_js"` in the engine's `.cargo/config.toml`. (Or feature-gate
   `shared-domain`'s `encryption` module out of the wasm build to drop the 0.2 dep.) Nothing configures
   this today.
4. Keep `chrono` default features (wasmbind) so `Utc::now()` routes to `js_sys::Date` — works on wasm.
5. (native ergonomics) split `shared-infrastructure` `local` → `local-core` (`sqlx/sqlite` +
   `LocalFsClient`) and `local-http` (axum/utoipa). The axum usage is isolated to ONE file
   (`files_storage/local_fs/blob_proxy.rs`), so a native local engine gets SQLite + `LocalFsClient`
   without a web framework.

**C. Code changes (leaks):**
1. `shared-domain/src/errors.rs:34` — `Sqlx(sqlx::Error)` is the one hard leak. Feature-gate or
   stringify the variant so `shared-domain` no longer unconditionally needs `sqlx`. Prerequisite for B1.
2. `shared-infrastructure/src/database/mod.rs:13` `SqlxTimestamp` alias — only bites if the engine links
   `shared-infrastructure` on wasm; B2 (don't link it on wasm) sidesteps it.

**D. New wasm adapters the engine writes (native reuses the sqlx/LocalFs equivalents):**
1. `ModelPersistence` over OPFS/IndexedDB (9 methods).
2. `FileStorageOps` + `FileStorageClientProvider` over OPFS (replaces the `tokio::fs` `LocalFsClient`;
   no `blob_proxy`/axum on wasm).
3. `EventBus` — a trivial synchronous local impl (the prod `in_memory` one uses `tokio::spawn`).
4. `ContextFetcher`, `ExperimentPersistence`, `PermissionService` — the remaining constructor ports.

**Composition-root note (scoping):** `ModelAppService::new` takes `PermissionService`,
`FileStorageClientProvider`, `ModelPersistence`, `ExperimentPersistence`, `ContextFetcher`, `EventBus`,
`Clock` — so "basic model operations" is more than a model store. Like station's fixed single-tenant
context, the engine supplies **local/fixed impls**: a permissive `PermissionService`, a fixed-identity
`ContextFetcher` (one local project/namespace/user), a trivial `EventBus`. `ExperimentPersistence` is
only for the promote-experiment-file-to-model path — defer it (and that op) if the first cut is
create/list/load/upload. Net: the wasm engine is the **inverse of `burn-station-api`** — no `local`
feature, no axum, custom OPFS + persistence adapters, slimmed tokio.

## 13. Next arc — the registry as metabolic's single model store (Library/Catalog redesign)

**Decision (user, 2026-07-17): extend full replacement to ALL models.** Base/pretrained checkpoints
downloaded from the catalog become registry models too; the registry becomes metabolic's **only** model
store; the UI splits into a **Library** (registry view) and a **Catalog** (discovery view); quantization
becomes a registry-native **fork**. Motivation: sync (Deliverable 2) — explicit push, a library that can
show synced-but-not-pulled entries next to local ones, and derived-model lineage that resolves inside one
namespace instead of via out-of-band hub strings.

**This revises M1 + M5 (§10).** Both were Deliverable-1 scoping calls; with sync coming, two catalogs
mean sync covers half the models and the UI stays split forever. What makes the reversal cheap is already
on disk: metabolic ports at download time, so a base cache dir holds exactly `model.bpk` +
`tokenizer.json` (`metabolic-models` import/pretrained.rs:76, hub.rs:65) — the same artifact set and the
same per-file mmap load seam as a registry version. Ingesting a base is "move two files and change who
tracks them", not a format change. M2/M3/M4 stand unchanged and are what make this possible (opaque
files, opaque metadata, bytes-not-models at the load boundary).

### Decisions

| # | Decision | Rationale |
|---|----------|-----------|
| L1 | **Catalog = discovery, Library = registry.** The static `REGISTRY` (and the browser manifest, and later the remote hub) is a discovery source whose "download" action is fetch → port → `publish`. The registry is the only runtime model catalog. | One mental model on both targets — generalizes the Phase-4 manifest-as-one-time-import decision to native. |
| L2 | **A not-yet-downloaded catalog entry is NOT a registry entry.** The entry is created at download/publish time; the engine invariant "every version's artifacts are present" holds until sync. | Keeps availability state out of this arc. "Metadata known, artifacts absent" arrives once, with sync, where it's unavoidable; catalog "not downloaded" = simply not in the library. |
| L3 | **Quantize = fork: a NEW model, not a new version.** Lineage is app-side metadata `derived_from: {model, version}`. Versions stay reserved for "same logical model, revised" (what sync's revisions/`latest` mean). | A quant is a different deployable with different runtime config. Fork lineage resolves inside the registry namespace — what a syncing peer needs. |
| L4 | **Origin is app-side metadata** (`origin: {hub_repo, revision?}`), and the quantizer consumes the ORIGIN, never the registry artifact: `quantize_streaming` streams the raw safetensors shards from hf-hub's cache at original dtype (import/pretrained.rs:113); the ported bpk is precision-cast and unsuitable. "Quantize from the registry" = the UI resolves the parent's origin and fetches shards (re-download if evicted). | The registry is the only *model* store; hf-hub's shard cache stays as transient import material, not a model store. |
| L5 | **Sync stays explicit-push.** `origin` later enables "push metadata only, peers re-materialize from origin" for imported bases, without a schema break. Do not promote `origin` to a domain field until sync policy actually reads it. | Avoids pushing multi-GB public checkpoints to the hub by default; keeps M3 opacity honest for now. |
| L6 | **Ingest-by-move is a prerequisite.** Publish stream-copies today; bases are the dense F16 artifacts. Port already writes temp + atomic rename, so the shape is port-into-staging → publish-by-rename/hard-link, hashing during the staging write. (Also closes §12 follow-up (2), the quantize double write.) | Without it every download pays a second multi-GB write + hash. Compatible with content addressing: hash, then link into place. |
| L7 | **Chat models first.** STT (Whisper) and T2I (SD3.5) live in the same checkpoint cache and should follow, but not in this pass. | Scope guard. |

Precision note: the base artifact is published at the ported (run) precision, recorded in the metadata
(already in the schema); loading at a different precision keeps today's runtime-cast behavior (bpk.rs:122).
A deliberate re-port at another precision would be a new *version* of the same model — a natural fit,
not needed now.

### The Library state model

Pre-sync every entry is **local**. Sync adds **synced** (pushed) and **remote-only / synced-not-pulled**
(metadata known, artifacts absent). The engine/facade gap this surfaces for Deliverable 2 — deliberately,
before the reconciler exists: per-version **artifact availability** plus a `pull`/materialize verb, with
`fetch` on an unhydrated version failing typed, not panicking. The Library UI ships now with the state
badge stubbed to "local".

### Phasing (continues the metabolic arc; Phases 0–5 are §12's)

- **Phase 6 — ingest-by-move (L6).** Local rename/hard-link fast path on blob-store publish (native;
  wasm keeps stream-copy).
- **Phase 7 — base ingestion (L1/L2).** Download+port ends in `publish` (metadata: origin, family,
  precision, display name); `is_downloaded` (embedded.rs:2374) becomes a registry lookup; "Remove from
  disk" becomes registry delete; adopt-on-init migration for existing base cache dirs (pattern proven in
  Phase 3). Retire the checkpoint-cache probe.
- **Phase 8 — quantize-as-fork (L3/L4).** `DerivedSpec.base_repo` becomes a registry model reference;
  the quantize form launches from a Library card; `derived_from` + `origin` written; shards fetched via
  the parent's origin.
- **Phase 9 — Library/Catalog UI split.** Library = registry view (base and derived alike, origin/quant
  badges, stubbed sync state); Catalog = discovery (`REGISTRY`, browser manifest, later the remote hub)
  with an explicit "add to library" download action (lazy download-on-first-load may remain as a
  convenience).
- **Then Deliverable 2** starts against a registry that is the single source of truth for everything:
  availability + `pull` + explicit push, per the state model above.

### Verification

- Phase 6: publishing a staged dir moves the file (same inode, no second full write); wasm unchanged.
- Phase 7: a fresh download lands in the registry and the catalog badge flips via registry lookup;
  reopen still lists it; delete removes blobs + catalog entry; legacy base cache dirs adopt idempotently;
  base load still mmaps via `local_path` with unchanged load time / peak memory.
- Phase 8: forking a library base yields a new model whose metadata carries `derived_from` + origin;
  the quantizer streams shards from the origin (re-downloads when hf-hub's cache is evicted; never reads
  the parent's bpk).
- Phase 9: the Library lists registry truth only; the Catalog derives in-library state from the
  registry, not the filesystem.

### Refined design (2026-07-17) — ingest-by-move (Phase 6) + origin/availability

Refinement of L6 and L4/L5 after reading the current publish/fetch code end to end. Positions marked
**[recommended]** are the proposed direction, pending a confirm before implementing.

**Throughline — these are one problem, not two.** §13 makes the registry metabolic's single model store, so
base checkpoints (dense F16, multi-GB) become registry entries (Phase 7). That makes both L6 and L4/L5
load-bearing at once: **ingest-by-move** so *putting a base in* doesn't cost a second multi-GB write, and
**origin** so *syncing a base out* doesn't push GBs of public bytes to the hub. Both reduce to one principle:
the content hash is identity; the bytes are either *movable* (ingest) or *reconstructible from source* (origin).

#### A. Ingest-by-move (Phase 6)

Today native publish stream-copies every file through `FsBlobStore::put` (tracel-engine `storage.rs`) —
read 64 KB → hash → write 64 KB — even though the producer (quantize/port) already wrote those exact bytes to
`staging/`. The second write is pure waste and it is the expensive half on a multi-GB base. The source path
already exists and is discarded one layer too early:

```
FsBundle.local_path(rel) = Some(abs_path)   (tracel-artifact bundle/fs.rs) — the path EXISTS here
  → Artifacts = Box<dyn BundleSource>        (local_path default None; FsBundle overrides)
  → BundleArtifactSource(Artifacts)          (tracel-engine registry.rs) — forwards only list()/open(); DROPS local_path
  → ArtifactSource { list, open }            (model-registry publish_model_version.rs) — no local_path method
  → blobs.put(reader)                        (BlobStreamPort) — stream-copy only
```

**The reshape — 4 changes across 2 crates (`model-registry` + `tracel-engine`); the SDK and metabolic are untouched:**
1. `ArtifactSource::local_path(rel) -> Option<&Path>`, default `None` (`model-registry`) — symmetric with the
   existing `local_path` on `BundleSource`/`BlobHandle`.
2. `BundleArtifactSource::local_path` forwards `self.0.local_path(rel)` (`tracel-engine`, one line).
3. `BlobStreamPort::ingest(at, from: &Path) -> BlobStat`, default impl = open+`put` (stream-copy fallback).
   `Path` is already in this port's vocabulary via `BlobHandle::local_path`, so no new coupling.
4. `FsBlobStore::ingest` overrides: `rename` into place, fall back to copy+unlink on any rename error (EXDEV / cross-fs).

Publish then chooses per file:
```rust
let stat = match source.local_path(&rel_path) {
    Some(path) => self.blobs.ingest(&location, path).await?,
    None       => self.blobs.put(&location, &mut *source.open(&rel_path)?).await?,
};
```

**metabolic gets it for free.** It already publishes via `FsBundle` (Phase 5), which already implements
`local_path`, and staging already sits beside the registry root on the same filesystem (Phase 5, "ready for
L6"). So `rename` is O(1) and the fast path lights up the moment the plumbing lands — no metabolic change.
wasm is unaffected: no `local_path` → always stream-copy into OPFS (matches L6).

**Checksum fork.** `put` hashes while streaming; `rename` moves without reading.
- **(a) Hash-on-ingest** — read once for the digest, then rename. 1 read, 0 writes (vs copy's read+write).
  Removes the expensive write, no producer change, store still owns the authoritative hash. **[recommended] ship first.**
- **(b) Source-carried digest** — producer hashes during the staging write (L6's literal phrasing); ingest is a
  pure rename, 0 reads/0 writes. Clean form: write staging *through* `BundleSink` (which already computes
  checksums, surfaced on `FsBundleFile.checksum`) rather than hand-rolled hashing. Adopt when content-addressing
  needs the hash up front anyway.

**Ingest semantics [recommended]: consume-by-rename.** `ingest` consumes `from` (renamed away / copied-then-removed).
One physical copy of the bytes ever, and it composes with future dedup: *if a blob with this hash already exists,
drop `from`; else move it.* Contract: **the caller must not read the staged file after ingest** — safe for
metabolic (post-publish it loads via `registry.fetch`, which points at the store, never back at staging).
Alternative considered and rejected: hard-link (non-consuming, safer against "file vanished," but leaves staging
for the sweep) — rejected in favor of the cleaner true-move.

Phase 6 is self-contained and blocks Phase 7 base ingestion; **build it first.**

#### B. Origin + availability

**The invariant sync breaks.** Today an implicit engine invariant (L2) holds: a version exists ⟹ its blobs are
present, so `fetch` always finds bytes. Sync introduces "metadata known, artifacts absent" — a peer learns a
version before (or without ever) receiving its bytes. Today that fails *untyped*: `fetch_model_version` opens
each blob and an absent one surfaces as `NotFoundWithCode("BLOB")` → SDK `Backend(..)`, indistinguishable from a
real failure. The Library UI must tell "pullable" apart from "broken."

**Availability is computed local state, not synced content.** Two peers disagree about the same version's
availability, so it is a local-replica fact (D3/D6) — never part of the shared domain model, and never a stored
flag that can drift. **Compute** it by probing the blob store ("are all manifest files present?"), which is what
fetch already does implicitly.

**Landable now (cheap, forward-compatible, never fires pre-sync):**
- typed `ModelRegistryError::ArtifactsUnavailable { name, version }` so fetch-on-absent is typed, distinct from
  `VersionNotFound` (metadata absent) and `Backend` (real failure);
- `BlobStreamPort::exists(at)` (or reuse `open`'s not-found) + an `availability(name, revision) -> Present | Absent`
  facade query;
- together these let the Library UI be written against real shapes today with the state badge stubbed to "local"
  (as the Library state model already anticipates).

**The `pull` verb (Deliverable 2) [recommended]: explicit, on the engine handle.** Materializing an Absent
version is a slow, gated, progress-bearing network op → `Engine::pull(name, revision)` on the handle alongside
`sync()` (D3: lifecycle, not the SDK domain facade). `fetch` stays "materialize locally-present artifacts, fail
typed if absent." This resolves a latent ambiguity: `ModelRegistryProvider::fetch`'s doc says "downloading if
needed" (`provider.rs`) — decided **against** transparent pull-on-demand.

**Origin as a re-materialization recipe.** For L5's "push metadata only, re-materialize from origin," origin must
be enough to reconstruct the artifacts: `origin { hub_repo, revision, port_params }`. The quantizer already
consumes origin, not the ported bpk (L4).

**Identity fork [recommended]: per provenance class.** Is a synced base identified by artifact content (bpk hash,
D4) or by origin (repo+revision+port params)?
- *Derived/owned* models: **content-addressed, byte-synced** — their bytes exist only because you made them, so
  you must ship them.
- *Imported bases*: **origin-addressed, metadata-only-synced, re-materialized on pull**, integrity anchored on
  HF's own safetensors sha (verifiable against HF), with the local bpk digest treated as a per-peer local
  detail. This avoids demanding bit-reproducible porting across heterogeneous hardware (the failure mode of pure
  content identity: "same base" splits into different hashes on different machines) and keeps multi-GB public
  bytes off the hub.

**L5-safe scaffolding (landable now).** Add a structured `origin { hub_repo, revision }` to `RegistryMeta`
(`metabolic-registry/meta.rs` — today it has only `source`, a geometry hint; no `origin`/`derived_from` exists in
the registry metadata surface yet). It stays opaque `Value` to the engine (honors M3/L5). When sync lands, the
sync strategy (in `model-registry` per Q11, not the SDK) starts *reading* that same field to choose push-mode +
route pull. Because metadata is already a persisted opaque blob, adding a reader later is purely additive — the
"promotion" is opaque-only-metabolic-reads → opaque-the-sync-strategy-also-reads, never a typed domain column.
Exactly the "no schema break" L5 asks for.

#### C. Identity model (Phase 8/9 substrate) — make the registry the identity source of truth

**The problem (surfaced reviewing Phase 7, 2026-07-17).** §13 says "the registry is metabolic's single model
store," but after Phase 7 the registry is *not yet the identity source of truth* — the static catalog
(`metabolic-inference` `REGISTRY`) + app-side id conventions are, and the registry **name** is a computed
projection hanging off them. No active bug (the app owns every write pre-sync, so the app-id → registry-name →
app-id round-trip always closes), but three latent incohesions that Phase 8/9/sync will stress:

1. **The base list is catalog-driven, not registry-driven.** `emit_models` iterates `catalog()` and consults
   the registry only as a side table keyed by repo (`ingested_bases.get(spec.repo)`). Off-catalog base repos
   are a supported concept (`architecture_for` has a family fallback), so an ingested base absent from the
   static catalog lands in the registry yet never lists. The list's source of truth is the catalog.
2. **The app-id ↔ registry-name map is a lossy per-kind projection at the boundary.** `base_registry_name`
   (`owner/name` → `owner--name`) for bases, `registry_name`/`derived_app_id` (`derived/x` ↔ `x`) for derived.
   Reversing needs `origin` as an out-of-band discriminator, and it is asymmetric (a base's app-id comes from
   `origin.hub_repo`, not from un-slugging the name). For a base the identity is duplicated: name (slugged repo)
   AND `origin.hub_repo` (exact repo), which must stay in lockstep.
3. **Base-vs-derived is decided two different ways.** `model_key`/`derived_model` decide by name-shape+lookup
   (`is registry_name(id) present?`); `emit_models` decides by `origin.is_some()`. Two encodings of one taxonomy.

**Decision — registry-canonical identity, resolved in ONE place.** The registry entry (name + metadata) is the
canonical identity; app-facing ids are *derived from* entries, never the reverse. Concretely:

- **C1 — explicit `kind` in `RegistryMeta`.** Add a tagged `kind: Base | Derived` (opaque `Value` to the engine,
  M3/L5-safe). This is the single discriminator — retire the `origin.is_some()` / name-shape tests (#3). `origin`
  becomes orthogonal: a `Base` carries `origin{hub_repo}` (re-fetch recipe = identity, L4/L5), a `Derived`
  carries `derived_from` lineage (Phase 8) and no origin. "Has a re-fetch recipe" and "is a base" stop being
  conflated.
- **C2 — one resolver, one direction.** Replace the scattered `base_registry_name`/`registry_name`/
  `derived_app_id`/`derived_model` logic with a single `resolve(app_id) -> RegistryEntry` (registry-backed) and
  its inverse `app_id_of(entry)` (the ONLY place that projects an entry to a UI id/label). The app-id *string*
  can stay what it is today (repo for base, `derived/<slug>` for derived) as a stable **alias** — the fix is not
  the string format, it is that the mapping is scattered and bidirectional-by-projection. Centralize it; make
  every read go through the registry.
- **C3 — list from the registry; catalog is discovery-only (realizes L1).** `emit_models` (Phase 9) iterates the
  registry for what you *have* (both kinds, `kind` tags them) and LEFT-JOINs the static catalog by
  `origin.hub_repo` for what you *could add*. A catalog entry with no registry match = available-to-download; a
  registry base with no catalog match = a user/synced import that still lists (geometry via the existing
  `architecture_for` family fallback). Fixes #1: an ingested off-catalog base becomes visible.
- **C4 — agents/preferences keep the app-id alias (no re-key migration now).** `agent.model`, avatar/star keys
  stay the app-id string, resolved through C2's resolver. Re-keying agents onto registry names is a bigger
  migration deferred until sync forces a stable-id question; the alias + single resolver removes the incohesion
  without it.
- **C5 — precision is NOT part of base identity (unchanged, documented).** One artifact per base repo, stored at
  first-ingested precision (`ensure_base_ingested` early-returns on name presence) — same "first precision wins"
  the old checkpoint cache had (keyed by repo, one `model.bpk`). A later request at another precision gets the
  stored one. Known limitation, not fixed here; if per-precision base variants become real, the registry *name*
  (not the app-id) grows a precision segment. Flag, don't fix.
- **C6 — cross-provider naming: DEFERRED to sync (Deliverable 2), scope note.** C1/C2 make identity coherent
  *within* the native provider. The browser provider is separately already 1:1 (manifest id = registry name =
  app id; `registry_name_for` would be a no-op there, so C2 is native-only by necessity, not omission — C1's
  `kind` *did* land in both). But native and browser use **different names for the same logical model** (native
  base `Qwen--Qwen3-0.6B` / derived bare-slug vs browser manifest slug `qwen3-0.6b-q4`). Harmless pre-sync
  (separate FS/OPFS registries, never compared); at sync it blocks by-name dedup across providers. The canonical
  cross-provider name is a Deliverable-2 decision — likely `origin.hub_repo`-derived for bases, content-digest
  for derived (byte-synced). Do NOT unify the two providers' naming now (speculative before the reconciler
  exists); the shared resolver would move out of `embedded.rs` when it does.
  - **C7 only HALF-fixes C6 (refinement, 2026-07-17).** After C7 the two providers share the adapter,
    `RegistryMeta`, and the core principle (id *is* the registry name; registry = store; `emit_models` =
    registry ∪ discovery source), and the browser is internally clean — arguably *simpler* (one model concept,
    id = manifest id = registry name = app id; no id functions; it doesn't quantize, so it never exercises the
    base/derived machinery and `kind` is set-but-unread there). But the base identity *substrate* genuinely
    differs: native bases are **origin-addressed** (`find_by_origin`), browser bases are **name-addressed**
    (`origin: None`) because their artifacts are **host-served** (a weights-host URL), not hub checkpoints — so
    the browser cannot origin-address them. Consequence for sync: C7's "match native bases by origin" does NOT
    bridge browser↔native (a browser base has no origin to match). The reconciler will need its own cross-provider
    key for host-served bases (content digest, or synthesize an origin from the manifest `source`). Still
    Deliverable-2; not unified in implementation, and correctly so for now.

**Why now.** Phase 8 (fork) needs stable lineage ids to point `derived_from` at; Phase 9 (Library view) forces
listing *from* the registry, which is exactly the inversion C3 describes; sync is the hard wall (a peer's entries
fit neither `base_registry_name`'s convention nor the local catalog). Pinning C1–C2 as the substrate *before*
Phase 8 means both phases build against registry-canonical identity instead of extending the projection. C1
(the `kind` field) is the cheap, additive, do-it-first piece; C2 (the resolver) is the mechanical consolidation;
C3 is Phase 9's listing rewrite.

**C1 + C2 DONE & test-verified (committed in `metabolic`, 2026-07-17).** C1: `ModelKind{Base,Derived}` on
`RegistryMeta`, set by every constructor (native base/derived, browser manifest — quantized→Derived,
dense→Base); `from_value` bridges pre-`kind` entries once by inferring from origin; `emit_models` splits on
`kind`, not `origin.is_some()`. C2: one `registry_name_for(app_id)` (subsumes `registry_name` +
`base_registry_name`, keyed on the `derived/` prefix) and one `app_id_of(name, meta)` inverse dispatched off
`kind`; `derived_model` gained the load-bearing `kind` filter (under the unified map a base repo now resolves to
its ingested entry, so the discriminator — not name shape — is what keeps derived lookups correct); every call
site routes through the pair. Opaque `Value` to the engine — metabolic-only, no tracel/burn-central change.
Verified: `metabolic-registry` 6 native + 1 wasm/OPFS (incl. legacy `kind`-inference + round-trip),
`metabolic-backend` 33 native (incl. name-map disjointness + `app_id_of` per-kind), both wasm leaves + clippy
clean. **C3 (list-from-registry) remains Phase 9.**

**C7 — full single-identity re-key DONE & test-verified (committed in `metabolic`, 2026-07-17). SUPERSEDES the
C2/C4 "centralize the conversion, keep the app-id alias" framing.** After review the user judged C2 a *converter*,
not an *elimination*, and chose to make the registry the genuine SoT. The change: (1) **Derived id IS its registry
name** — the `derived/` prefix is dropped; `derived_id` = bare slug; meta/fetch/delete/publish address the registry
directly, no conversion; `derived_from` points at the parent's app-id (base hub repo), portable for sync. (2)
**A base's identity of record is `origin.hub_repo`** — resolved by *querying* the registry
(`Registry::find_by_origin`), never by a name computed from the repo; **naming is a write-time concern only**
(`base_publish_name` mints a legal, *incidental* storage handle at publish — nothing parses it back or looks up by
it). So `registry_name_for`/`derived_app_id` are gone; the read-time forward-map dissolved. This also fixes
**C6-for-bases** (a peer's base under a different name still matches by origin) and de-risks `derived_from`
(references the portable app-id, not a local computed name). (3) **Store migration** strips the retired `derived/`
prefix from agent bindings + per-model settings (the registry never carried it, so nothing there moves;
idempotent, collision-safe). Browser unaffected (already 1:1). What survives from C1/C2: `kind` (the discriminator)
and `app_id_of` (the inverse projection) — both honest. What dissolved: the computed forward map + the prefix.
Verified: `metabolic-registry` 7 native + 1 wasm/OPFS (adds `find_by_origin`), `metabolic-backend` 36 native
(adds derived-id-is-slug, origin-resolution idempotency, prefix-strip migration), server/cli + both wasm leaves +
clippy clean. **C3 (list-from-registry) still Phase 9.** Next: Phase 8 fork on this substrate.

#### Decisions to confirm before implementing
- **Checksum:** (a) hash-on-ingest now → (b) source-carried later. **[recommended]** — *n/a, ingest reverted.*
- **Ingest semantics:** consume-by-rename, copy+unlink fallback. **[recommended]** — *reverted; revisit with CAS.*
- **Base identity under sync:** origin-per-class (base = origin-addressed/metadata-only; derived =
  content-addressed/byte-synced). **[recommended, confirmed]**
- **Pull:** explicit `Engine::pull`, not transparent `fetch`. **[recommended, confirmed]**
- **Identity model (C):** registry-canonical, explicit `kind`, one resolver, list-from-registry. **[recommended,
  user-confirmed 2026-07-17]** — C1 first (additive), then C2, then C3 with Phase 9.

#### State (2026-07-17)
Phase 6 landed availability/origin scaffolding; ingest-by-move was reverted (re-add with CAS). Phase 7 (base
ingestion) is implemented + test-verified. Phase 8 backend (`derived_from` fork lineage) is done. The identity
substrate is done and went deeper than C1/C2: **C7 — full single-identity re-key** (derived id == registry name;
base resolved by origin query; naming is write-time-only) makes the registry the genuine SoT, superseding the
C2/C4 converter approach. **Immediate next step: Phase 9 (Library/Catalog UI split) — realizes C3
(list-from-registry) + the fork's Library-card launch (resolve parent `origin.hub_repo`) + folds in the deferred
Phase 7 runtime verification.** Defer `Engine::pull` transfer + sync strategy to Deliverable 2.

## 14. Next domain — inference monitoring (local-first, engine-owned, dual-render)

**Status: exploration/design, 2026-07-21.** Refines the §4 line-110 "Inference telemetry" row and the
§5 "Later" bullet into a concrete vertical. Grounded in metabolic as the real consumer (like §10).
Picked as the next migration domain for a specific reason: it delivers value **native-first**, and —
counterintuitively — it is *more* wasm-tractable than the registry was, so it advances the metabolic
migration **without** committing to the SDK-wide async/wasm pivot the team is not ready for.

### 14.1 Nothing here is greenfield — three stacks already exist

| | metabolic | tracel SDK | burn-central backend |
|---|---|---|---|
| What | live local stat panel (chat UI) | `InferenceSession` + `InferenceSink` + cloud provider | inference-group telemetry sink + console dashboard |
| Model | capability-typed events -> fold -> derived rates | generic gauge/counter/distribution samples + metadata bag | gauge/counter/distribution in Postgres |
| Scope | one exchange, ephemeral, in-memory | per-request session w/ `InferenceId` | per named group, persisted, queryable |
| Transport | SSE `BackendEvent` to its own egui | `reqwest::blocking` batched POST | `POST .../inference-groups/{name}/telemetry` |
| Render | egui panels, live | fire-and-forget (no local view) | dashboard + log explorer (commit `2f7480a4b`, #1479) |

The SDK `MetricKind`/`MetricSample`/`MetricDescriptor` are a 1:1 match with the backend ingestion
payload — the SDK->backend cloud path is wired end-to-end. **The hole:** the *local-first* path does
not exist. `Connection::Offline` wires `DefaultInferenceProvider` -> `NoopSink`, whose own doc says
"it ships nothing … metrics/logs are discarded." The experiment side has `experiment/local.rs`
(writes to disk); the inference side has only `cloud.rs` + `mod.rs`. And `tracel-engine` is
registry-only (no inference surface). metabolic, meanwhile, has a rich producer but it is ephemeral —
`EngineStats`/`SessionStats` are in-memory `Vec`s that clear on model switch, never persisted, never
synced.

### 14.2 Design — engine-owned local vertical, dual-render

- **A new inference-telemetry vertical in `tracel-engine`**, as its OWN hexagonal vertical
  (domain: append-only sample log + descriptors + group logs; application: append + query/aggregate
  use-cases; infra: persist via the existing `storage/{fs,opfs}` seam). Honors revised **D8** — NOT a
  `DomainSync` strategy trait plugged into a shared toolkit (the §4 sketch is superseded); share
  transport/blob infra only by rule-of-three.
- **A local `InferenceSink`** — the missing sibling of `experiment/local.rs` — persisting into that
  vertical instead of `NoopSink`/cloud POST. Wire `Connection::Offline` (and an engine-backed
  connection) to it.
- **Dual-render (the recommendation):** metabolic KEEPS its live capability-typed panel unchanged;
  the engine vertical runs *alongside* it, adding the two things metabolic lacks — durable local
  history + a sync path. metabolic's producer feeds both; the engine sink is a second consumer of the
  same `BackendEvent` stream. Rejected "engine-owns-and-renders" (would flatten metabolic's rich
  derive-on-read model into generic samples and demote its live UX for no gain).
- **Sync deferred** to Deliverable 2+: append-only samples, aggregate/union — a *sibling* to the
  registry's sync living in this vertical's own layers, not a reuse of the registry ref-merge.

### 14.3 Session boundary — grounded in metabolic

The SDK session ends "when the output finishes" (one `infer()`-scoped request; `SessionStatsObserver`
fires `on_finish`). metabolic's reply is a `for round in 0..MAX_ROUNDS` loop (`MAX_ROUNDS = 4`,
`metabolic-backend/src/embedded.rs:1141`): each `run_round` is one decode run (Prefill -> Step* ->
outcome); tool-call rounds loop again; the **first non-tool round emits `TextGenerationEvent::Done`
and returns** (`embedded.rs:1150`).

- **=> one SDK `InferenceSession` == one metabolic *exchange*** (the whole reply). The tool rounds are
  **sub-runs within** the session, tagged `round=n` via `session.with_attributes`.
- **Use `InferenceSession` directly — NOT `Inference`/`InferenceJob`/`session.run`.** That machinery
  is the SDK *driving* inference (owns input->output, spawns a worker, applies the auto observer).
  metabolic drives its own engine loop over its own SSE bus with tool execution interleaved; routing
  it through `InferenceJob` would double-drive (SDK worker wrapping metabolic's pump) and duplicate
  the output path. Take the session, leave the job. `InferenceSession` is standalone by design (`run`
  is documented as *one* optional driver).
- **Adapter lives at the backend event seam** (where `Done` is synthesized), not in the decode loop:
  open a session on the round-0 Prefill (footprint -> scoped attrs); fold each round with metabolic's
  EXISTING `TextGenerationMetrics` and emit derived rates as `session.log_*` tagged `round=n`; close
  on final `Done`, or on early return (cancel/error) with `cancelled=true`.
- **SDK gap:** `InferenceModule` exposes `create(name, inf) -> InferenceJob` but **no**
  `create_session(name) -> InferenceSession` for observe-only use (`create_session` is on the provider
  trait, unsurfaced). Add it — it is the exact API the dual-render path needs.

### 14.4 The deep fork — where aggregation lives

The cloud model punts reduction to the server (`quantile_exact_weighted` plpgsql; "quantiles computed
server-side"). **Local-first has no server, so aggregation must be client-side** (engine or producer).
This is where metabolic's design is the *better* one: it transports raw events and derives rates once
on read (one honest "tokens/sec"); the SDK sink has no fold primitive. Fork: **(a)** the engine grows
a local aggregation layer, or **(b)** the SDK adopts metabolic's event-fold for the local sink. Lean
**(b)** — the honest unification, and it reuses metabolic's proven fold rather than reimplementing
quantiles server-style on the client.

### 14.5 Wasm posture — why this domain, now

The SDK telemetry transport today (`reqwest::blocking` + `std::thread` + `tungstenite`) is native-only,
zero wasm handling — that transport IS the pivot the team is deferring. **Local-first sidesteps all of
it:** hot path = a sync fire-and-forget enqueue (pivot-rule: fire-and-forget stays sync on wasm);
persistence = OPFS (already wasm-native and test-passed for the registry); no HTTP on the hot path.
Only sync-to-hub touches the cloud transport, and it is deferrable. Net: native metabolic works today;
browser metabolic reuses the proven OPFS seam; neither needs the async pivot. When the pivot does land,
fire-and-forget REST telemetry is also the *easiest first* wasm transport to port.

### 14.6 What the SDK/stack is missing (the gap list this surfaces)

1. **Local inference-telemetry vertical** in the engine (the big build; §14.2).
2. **Local `InferenceSink`** (`inference/local.rs`) — take `Connection::Offline` off `NoopSink`.
3. **Client-side aggregation** — no fold/derive primitive today (§14.4).
4. **Bare-session API** — `module.create_session` for observe-only (§14.3).
5. **One telemetry contract** — cloud (`inference/request.rs`) and fleet (`fleet/request.rs`) already
   duplicate `IngestTelemetryRequest`/`MetricData`; a local sink would be a third. Unify before three.
6. **No durability on the plain cloud path** (only the fleet WAL has it) — local persist gives it free.
7. **wasm sync transport** — deferred (Deliverable 2+).

### 14.7 Metric mapping (metabolic -> generic samples)

| metabolic (`metabolic-metrics`) | sample |
|---|---|
| `decode_rate` / `live_rate` | gauge `tokens_per_second` |
| `Prefill.duration` (== TTFT) | distribution `prefill_ms` |
| `Step.duration` | distribution `decode_step_ms` |
| `prompt_tokens`, `reused`, `tokens` | counters |
| `cache_used` / `cache_capacity` | gauges |
| `ModelFootprint.weights_bytes` / `kv_cache_bytes` / pool | gauges |
| `ModelFootprint{repo,device,precision,parameters}`, `capabilities`, `round` | session metadata (dimensions) |

### 14.8 Phasing (suggested)

- **P0 — native adapter (prove the shape):** engine-local sink + the backend-seam adapter; dual-render;
  drives the local vertical end-to-end with metabolic's real producer. No wasm.
- **P1 — local aggregation + query:** the §14.4 decision; render history from the engine (not just the
  ephemeral panel).
- **P2 — browser/OPFS:** reuse the registry's OPFS path; browser metabolic persists + renders locally.
- **P3 — sync-to-hub:** append-only-samples domain (aggregate/union) into the existing hub, reusing the
  Deliverable-2 transport where shapes match.
