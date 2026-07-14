# Handout: SDK Dataset Integration — resume in a new session

This file exists so a fresh Claude session can pick this work up with full
context. Point Claude at this file first.

## What this is

Implementing the GitHub issue: **"SDK: Dataset integration — Implement a burn
integration using the Dataset trait and the burn-station dataset streaming
API."**

- Design spec: `../docs/superpowers/specs/2026-07-14-dataset-integration-design.md`
  (relative to this file, i.e. `Documents/Programmation/Tracel/docs/superpowers/specs/...`)
- Implementation plan: `../docs/superpowers/plans/2026-07-14-dataset-integration-plan.md`
  (i.e. `Documents/Programmation/Tracel/docs/superpowers/plans/...`)

Both were produced via the `superpowers` brainstorming/writing-plans skills and
were approved by David before implementation started.

## Execution rules (non-negotiable, set by David)

1. **NEVER run `git commit` for me, in either the `sdk` repo or the
   `burn-station-demo` repo.** Every commit step written into the plan is to
   be skipped. David reviews the diff and commits it himself.
2. **Ask permission before proceeding to the next step**, and before starting
   each new task. Do not chain steps together without a check-in. This
   applies at both granularities: before moving to a new numbered Task, and
   before moving to the next Step within a Task.
3. Chosen execution mode: **Option 2, inline execution** (via
   `superpowers:executing-plans`), not subagent-driven — run in this session,
   not dispatched to background subagents.
4. Do not start implementation work on `main`/`master` without explicit
   consent first (standard `executing-plans` rule, reiterated here because
   Task 2 touches a repo that was on `main` when this was written).

## Repo / branch state as of 2026-07-14

- `sdk` repo: on branch `feat/add-dataset-modules`, clean working tree. Safe
  to implement Task 1 directly here.
- `burn-station-demo` repo: on branch `main`, clean working tree. **Before
  Task 2 touches this repo, ask David whether to create a feature branch
  first or whether editing `main` directly is fine.** This question was
  raised but not yet answered as of when this handout was written.

## Progress so far

- [ ] Task 1: `StationDataset<T>` — Burn `Dataset` adapter in `tracel-core`
  (not started)
- [ ] Task 2: Propagate the `burn` feature through the `tracel` meta-crate and
  migrate `burn-station-demo` (not started)

Update the checkboxes above (and note any deviations from the plan) as work
progresses, so the next session/handout stays accurate.

## Quick context recap (for the next session's own understanding)

- `tracel-core::dataset` already has `DatasetProvider` / `DatasetModule`
  (raw byte-page streaming from Burn Station) and `AnnotationItem`. Nothing
  implements `burn::data::dataset::Dataset` on top of it yet.
- `burn-station-demo/src-tauri/src/training.rs` currently hand-rolls this as
  a private `AnnotationDataset` struct — one HTTP round-trip per `get()` call,
  full page-walk on every `len()` call. This plan replaces that with a
  reusable, cached `StationDataset<T>` in the SDK, then migrates the demo onto
  it.
- Full technical rationale, alternatives considered, and exact code are in
  the spec and plan files linked above — read those in full before writing
  any code.
