# Burn Central Workspace

A core workspace library for Burn Central project management, code generation, and platform integration.

## Overview

`burn-central-workspace` provides the foundational functionality for working with Burn Central projects. It can be used as a library in other applications or as the foundation for CLI tools, web services, or other integrations.

## Features

- **Project Management**: Discover, load, and manage Burn Central projects
- **Code Generation**: Generate and manage code artifacts for projects
- **Function Discovery**: Analyze Rust code to discover trainable functions
- **Job Execution**: Run training and inference jobs locally or remotely
- **Client Integration**: Connect to the Burn Central platform
- **Configuration Management**: Handle application and user configuration
- **Compute Providers**: Interface with different compute environments

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
burn-central-workspace = "0.1.0"
```

### Basic Example

```rust
use burn_central_workspace::{BurnCentralContext, Config, Environment, ProjectContext};

fn main() -> anyhow::Result<()> {
    // Create a configuration
    let config = Config {
        api_endpoint: "https://heat.tracel.ai/api/".to_string(),
    };
    
    // Create and initialize context
    let context = BurnCentralContext::new(&config, Environment::Production).init();
    
    // Discover a project in the current directory
    let project = ProjectContext::discover(Environment::Production)?;
    
    // Create a client if credentials are available
    if context.has_credentials() {
        let client = context.create_client()?;
        // Use client for API operations...
    }
    
    Ok(())
}
```

### Function Discovery

```rust
use burn_central_workspace::tools::function_discovery::FunctionDiscovery;
use std::path::Path;

fn discover_functions(project_path: &Path) -> anyhow::Result<()> {
    let discovery = FunctionDiscovery::new(project_path);
    let functions = discovery.discover_functions()?;
    
    for function in functions {
        println!("Found function: {}", function.name);
    }
    
    Ok(())
}
```

### Code Generation

```rust
use burn_central_workspace::generation::GeneratedCrate;
use burn_central_workspace::entity::projects::ProjectContext;

fn generate_code(project: &ProjectContext) -> anyhow::Result<()> {
    // Load functions from the project
    let registry = project.load_functions()?;
    
    // Generate code based on the functions
    // (Implementation details depend on your specific use case)
    
    Ok(())
}
```

### Job Execution

#### Local Execution

```rust
use burn_central_workspace::{BurnCentralRunner, LocalRunOptions};
use std::collections::HashMap;

fn run_training_locally(
    runner: &BurnCentralRunner,
    function_name: &str,
) -> anyhow::Result<()> {
    // Configure execution options
    let mut overrides = HashMap::new();
    overrides.insert("epochs".to_string(), serde_json::Value::Number(10.into()));
    
    let options = LocalRunOptions::new()
        .with_overrides(overrides)
        .with_config_file("config.json".to_string());
    
    // Execute the training function
    let result = runner.run_training_local(function_name, options)?;
    
    if result.success {
        println!("Training completed successfully!");
        if let Some(output) = result.output {
            println!("Output: {}", output);
        }
    } else {
        println!("Training failed: {:?}", result.error);
    }
    
    Ok(())
}
```

#### Remote Execution

```rust
use burn_central_workspace::{BurnCentralRunner, RemoteRunOptions};

fn run_training_remotely(
    runner: &BurnCentralRunner,
    function_name: &str,
    compute_provider: &str,
) -> anyhow::Result<()> {
    let options = RemoteRunOptions::new()
        .with_code_version("v1.0.0-abc123".to_string())
        .with_backend(burn_central_workspace::generation::backend::BackendType::Wgpu);
    
    // Submit job to remote compute provider
    let result = runner.run_training_remote(function_name, compute_provider, options)?;
    
    if result.success {
        println!("Remote training completed!");
    } else {
        println!("Remote training failed: {:?}", result.error);
    }
    
    Ok(())
}
```

#### Complete Execution Example

```rust
use burn_central_workspace::{BurnCentralContext, BurnCentralRunner, Config, Environment, ProjectContext};

fn main() -> anyhow::Result<()> {
    // Initialize context and project
    let config = Config {
        api_endpoint: "https://heat.tracel.ai/api/".to_string(),
    };
    let context = BurnCentralContext::new(&config, Environment::Production).init();
    let project = ProjectContext::discover(Environment::Production)?;
    
    // Create runner
    let runner = BurnCentralRunner::new(&context, &project);
    
    // List available functions
    let training_functions = runner.list_training_functions()?;
    println!("Available functions: {:?}", training_functions);
    
    // Execute first available function locally
    if let Some(function_name) = training_functions.first() {
        let options = LocalRunOptions::new();
        let result = runner.run_training_local(function_name, options)?;
        println!("Execution result: {:?}", result.success);
    }
    
    Ok(())
}
```

### Compute Provider Integration

```rust
use burn_central_workspace::{BurnCentralContext, BurnCentralRunner, ProjectContext};

// For compute providers that want to execute Burn Central jobs
fn compute_provider_example(job_params: &str) -> anyhow::Result<()> {
    // Parse job parameters
    let params: serde_json::Value = serde_json::from_str(job_params)?;
    
    // Initialize context and project
    let config = burn_central_workspace::Config {
        api_endpoint: params["api_endpoint"].as_str().unwrap().to_string(),
    };
    let context = BurnCentralContext::new(&config, burn_central_workspace::Environment::Production).init();
    let project = ProjectContext::discover(burn_central_workspace::Environment::Production)?;
    
    // Execute the job
    let runner = BurnCentralRunner::new(&context, &project);
    let options = burn_central_workspace::LocalRunOptions::new();
    let result = runner.run_training_local(
        params["function"].as_str().unwrap(),
        options,
    )?;
    
    if result.success {
        println!("Job completed successfully");
    } else {
        println!("Job failed: {:?}", result.error);
    }
    
    Ok(())
}

// Or use the built-in compute provider main function
fn use_builtin_compute_provider() -> anyhow::Result<()> {
    // This handles the complete compute provider workflow
    burn_central_workspace::compute_provider::compute_provider_main()
}
```

## Architecture

The library is organized into several main modules:

- **`context`**: Core context management and client creation
- **`entity`**: Project and experiment entity management  
- **`execution`**: Job execution (local and remote)
- **`generation`**: Code generation utilities
- **`tools`**: Various utilities for Cargo, Git, and function discovery
- **`config`**: Configuration management
- **`app_config`**: Application-level configuration and credentials
- **`compute_provider`**: Integration for compute provider runtimes

## Environment Support

The library supports multiple environments:

- **Production**: Default production environment
- **Development**: Development environment for testing

## Error Handling

The library uses `anyhow::Result<T>` for error handling, providing rich error context and easy error propagation.

## Use Cases

### CLI Applications
Build command-line tools that can manage projects, discover functions, and execute jobs:
```rust
use burn_central_workspace::{BurnCentralContext, BurnCentralRunner};
// Full access to all library functionality
```

### Web Services  
Create web APIs that can trigger training jobs and manage projects:
```rust
use burn_central_workspace::{BurnCentralRunner, RemoteRunOptions};
// Execute jobs on remote compute providers
```

### Compute Providers
Integrate the library into compute infrastructure to execute Burn Central jobs:
```rust
use burn_central_workspace::compute_provider;
// Built-in compute provider functionality
```

### Desktop Applications
Build GUI applications for project management and job monitoring:
```rust
use burn_central_workspace::{ProjectContext, BurnCentralContext};
// Project discovery and management
```

### CI/CD Pipelines
Automate training and testing workflows:
```rust
use burn_central_workspace::{BurnCentralRunner, LocalRunOptions};
// Automated job execution
```

## Examples

The library includes several comprehensive examples:

- **`basic_usage.rs`**: Introduction to core functionality
- **`library_integration.rs`**: Third-party application integration
- **`execution_example.rs`**: Job execution demonstrations
- **`compute_provider_example.rs`**: Compute provider integration patterns

Run examples with:
```bash
cargo run --example basic_usage -p burn-central-workspace
cargo run --example execution_example -p burn-central-workspace
```

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.