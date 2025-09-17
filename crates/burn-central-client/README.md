_Work in Progress_

# Burn Central Client

Burn Central client for training, tracking, inference etc...

## Architecture

The client is organized into several key modules:

### Bundle Module (`bundle/`)

Core abstractions for handling bundles of files:

- **`BundleEncode`/`BundleDecode`** - Traits for encoding/decoding data to/from file bundles
- **`BundleSource`/`BundleSink`** - Traits for reading/writing files in bundles  
- **`MemoryBundleReader`** - In-memory bundle reader for cached/synthetic bundles
- **`BundleSources`** - Builder for creating bundles with multiple files

### Artifacts Module (`artifacts/`)

Experiment-scoped artifact operations built on bundle abstractions:

- **`ArtifactScope`** - Upload/download artifacts within a specific experiment
- **`ArtifactKind`** - Types of artifacts (Model, Log, Other)
- Re-exports bundle traits for convenience

### Models Module (`models/`)

Project-scoped model operations built on bundle abstractions:

- **`ModelRegistry`** - Registry interface for downloading models
- **`ModelScope`** - Operations on a specific model within a project
- **`ModelVersionInfo`** - Metadata about model versions
- Re-exports bundle traits for convenience

### Key Benefits

1. **Clear Separation**: Artifacts (experiment-scoped) vs Models (project-scoped)
2. **Reusable Abstractions**: Core bundle handling can be used for both artifacts and models
3. **Clean APIs**: No confusion between artifact-specific and model-specific operations
4. **Extensibility**: Easy to add new bundle-based features

## Usage

```rust
use burn_central_client::BurnCentral;

let client = BurnCentral::from_env()?;

// Working with artifacts (experiment-scoped)
let artifacts = client.artifacts("owner", "project", 123)?;
artifacts.upload("checkpoint", ArtifactKind::Model, my_model, &settings)?;

// Working with models (project-scoped) 
let models = client.models();
let model = models.download(ModelPath::new("owner", "project", "my_model"), 1, &settings)?;
```
