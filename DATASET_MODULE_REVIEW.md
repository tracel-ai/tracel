# Dataset Module — Architecture Review

Branch: `feat/add-dataset-modules`. Scope: `crates/tracel-core/src/dataset/*`,
`connection.rs`, `context.rs`, `lib.rs`, `crates/tracel/src/lib.rs`, `Cargo.toml`.

Findings are ordered most → least important.

## 1. `AnnotationDataset` violates burn's `Dataset` contract and can silently truncate training data

**Files:** `crates/tracel-core/src/dataset/burn.rs` (`get`, `len`)

`burn_dataset::Dataset::iter()` builds a `DatasetIterator` whose `next()` is:

```rust
fn next(&mut self) -> Option<I> {
    let item = self.dataset.get(self.current);
    self.current += 1;
    item
}
```

Standard `Iterator` semantics mean the **first `None` ends iteration**, even if
`current < len()`. Two things in `AnnotationDataset` can produce that `None`
(or a wrong item) for a valid index:

- `get()` fetches the page, filters out malformed JSON items, then does
  `.nth(offset)` where `offset = index % page_size` was computed **before**
  filtering. If any earlier item in the same page is malformed, `offset` now
  points past the true match: `get()` either returns the *wrong* item (shifted
  by however many were skipped) or `None` if the shift runs the filtered
  sequence out.
- `len()` walks all pages once and caches the count in a `OnceLock`, but on any
  transient network/deserialization error it just `break`s out of the loop
  (`let Ok(page) = ... else { break; }`) and caches whatever partial count it
  had — a transient failure permanently under-reports the dataset size for the
  life of the `AnnotationDataset`.

Net effect: one malformed record or one blip during the `len()` walk can
silently stop a training run partway through the dataset with no error
surfaced anywhere — the loop just looks like it "finished early." This is the
kind of bug that's very hard to notice in practice (loss curves just look like
training saw less data) and directly undermines the reason this trait exists.

**Fix direction:** decode the whole page once, keep the *entry_idx* (not the
in-page filtered position) as the source of truth for indexing, and propagate
I/O errors instead of treating them as end-of-data.

## 2. `Cargo.toml` regresses the feature-gating work this branch is built on top of

**File:** `crates/tracel-core/Cargo.toml`

```toml
[features]
-default = []
+default = ["station"]
 station = ["tracel-client/station"]
```

plus `burn.workspace = true` added as a **mandatory** dependency (no
`dep:burn` / `burn` feature at all anymore, unlike the earlier
`feat: add feature gates` commit which had `burn = ["dep:burn"]`).

The root `Cargo.toml` depends on `tracel-core` without `default-features =
false` (`tracel-core = { path = "crates/tracel-core", version = "0.7.0" }`),
and `crates/tracel/Cargo.toml` also pulls in `tracel-core.workspace = true`
with no `default-features = false`. Because Cargo unifies features across the
build graph, this means:

- Every consumer of `tracel-core` — and transitively every consumer of
  `tracel` — now compiles in the Station HTTP client and the entire `burn` ML
  framework by default, regardless of whether they touch datasets at all.
- `tracel`'s own `station` feature flag (`station = ["tracel-core/station"]`)
  becomes a no-op: `tracel-core`'s default already turns `station` on, so
  disabling `tracel`'s `station` feature no longer removes it from the build.

This directly undoes the intent of the prior `feat: add feature gates` commit
on this same file, and adds real build cost (compiling `burn` + its backends)
for users who only want cloud/offline experiment tracking.

**Fix direction:** keep `default = []` on `tracel-core`, make `burn` an
optional dependency gated by its own feature (as the earlier commit had it),
and gate `mod dataset` / `pub use dataset::*` behind that feature so the
public API doesn't leak burn types when the feature is off.

## 3. Not-found error path is wrong for any project with >10 datasets or versions

**File:** `crates/tracel-core/src/dataset/station.rs` (`ensure_dataset_exists`,
`ensure_dataset_version_exists`)

Both helpers call `query()` / `versions()` with `QueryDatasetsRequest::default()`
/ `QueryDatasetVersionsRequest::default()` (i.e. `page: None, per_page: None`),
and the backend defaults `per_page` to **10** when unset
(`../backend/backend/crates/dataset/src/application/services/registry.rs:87,141`).
`ensure_dataset_exists` then checks only
`response.items.iter().any(|d| d.name == name)` — the first page.

If the dataset (or version) that legitimately exists isn't among the first 10
returned, this reports `DatasetNotFound` / `VersionNotFound` for a dataset
that's actually there. Since this path only runs after the primary
`stream_items` call already failed with `NotFound`, the practical effect is:
users with more than 10 datasets/versions in a project get a misleading
"doesn't exist" error instead of whatever the real problem was.

**Fix direction:** paginate through all pages (or pass a name/version filter
to the query if the API supports one) before concluding not-found.

## 4. No page caching — random-access training re-fetches a full page per item

**File:** `crates/tracel-core/src/dataset/burn.rs` (`get`)

Every call to `get(index)` issues a fresh `stream_items` network request for
the whole containing page (256 items by default) and only uses one of them.
Burn's typical dataloader usage pattern is **shuffled random access** over the
full index range — exactly the case this design handles worst: iterating a
dataset of N items in shuffled order does on the order of N network round
trips, each pulling `page_size` items, i.e. up to `page_size`× more data
transferred than the dataset actually contains. The type's own doc comment
acknowledges this ("no caching is done yet") but it's worth flagging as an
architectural gap rather than a nit, since it changes this from "usable
Station-backed dataset" to "usable only for sequential/small-scale access."

**Fix direction:** cache the last fetched page (page-start → items) behind a
`Mutex`/`RwLock` so consecutive shuffled indices that land in the same page
don't refetch.
