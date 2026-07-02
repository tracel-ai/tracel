# Handout: "Allow downloading model artifact from model registry"

Context for picking this back up in another conversation.

## Where things stand

- `ModelInfo` and `ModelInfoVersion` are already implemented in this module.
- There is **no single "ModelArtifact" object on the backend** — a model version's
  downloadable content is a *manifest of files*, not one artifact entity.
- The download flow already works end-to-end via
  `ModelRegistryProvider::download_plan` (`mod.rs:36`), implemented in `cloud.rs`,
  which returns `Vec<tracel_artifact::download::ArtifactDownloadFile>`.

## Backend shape (for reference, do not need to change)

- `ModelVersion` (`backend/backend/crates/model-registry/src/domain/model/mod.rs`)
  has a `manifest: ModelVersionManifest { files: Vec<BillableFileDescriptor> }`.
- `BillableFileDescriptor`: `rel_path`, `blob_key`, `size_bytes`, `checksum`.
- Download endpoint: `GET .../models/{model_name}/versions/{version}/download`
  → `ModelDownloadResponse { files: Vec<PresignedModelFileUrlResponse> }`
  where each file has `rel_path`, `url`, `size_bytes`, `checksum` (presigned URLs,
  not streamed bytes).

## Client shape

- `client/tracel-client/src/model/mod.rs::presign_model_download` wraps the
  endpoint above.
- `client/tracel-client/src/model/response.rs::PresignedModelFileUrlResponse`
  only keeps `rel_path` + `url` (drops `size_bytes`/`checksum` even though the
  backend sends them).

## SDK shape (current)

- `sdk/crates/tracel-core/src/model_registry/cloud.rs` maps the client response
  into `ArtifactDownloadFile { rel_path, url, size_bytes: None, checksum: None }`.

## Decision: `None` for `size_bytes`/`checksum` is intentional, not a gap

This mirrors the existing convention used for experiment artifact downloads:
- `sdk/crates/tracel-core/src/experiment/remote/cloud/mod.rs:151-178`
  (`Artifact::download`) does the exact same thing — hardcodes `size_bytes: None,
  checksum: None` when building `ArtifactDownloadFile`, even though the client's
  `presign_artifact_download` response *does* carry those fields
  (`client/tracel-client/src/artifact/response.rs`).
- Same pattern again in `sdk/crates/tracel-core/src/experiment/remote/station/mod.rs:157-158`.
- `ArtifactDownloadFile` itself defaults these fields to `None` in places in
  `sdk/crates/tracel-artifact/src/download.rs:267-274` and
  `sdk/crates/tracel-artifact/src/bundle/fs.rs:92-93`.

These two fields (`size_bytes`, `checksum`) exist on `ArtifactDownloadFile` for
multi-device/fleet verification use cases (out of scope for now). For
single-instance download — which is all we're implementing right now — `rel_path`
+ `url` is sufficient, and `None`/`None` is the established house pattern, not a
bug to fix.

## Open question for next session

Confirm whether the model_registry module still needs a `download()` method
analogous to `Artifact::download` in
`sdk/crates/tracel-core/src/experiment/remote/cloud/mod.rs:151-178` (fetch →
presign → build `Vec<ArtifactDownloadFile>` → `download_artifacts_to_sink` →
return a bundle), or whether `download_plan` alone satisfies the story.
