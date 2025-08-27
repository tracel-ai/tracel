use std::io::BufReader;

use crate::{
    commands::training::local_run_internal,
    config::Config,
    context::CliContext,
    entity::projects::ProjectContext,
    generation::backend::BackendType,
    tools::{cargo, functions_registry::FunctionRegistry, terminal::Terminal},
};

#[derive(serde::Deserialize)]
pub struct BUUU {
    /// The training function to run. Annotate a training function with #[burn(training)] to register it.
    function: String,
    /// Backend to use
    backend: BackendType,
    /// Config file path
    config: Option<String>,
    /// Batch override: e.g. --overrides a.b=3 x.y.z=true
    overrides: Vec<(String, serde_json::Value)>,
    /// Project version
    project_version: String,
    namespace: String,
    project: String,
    key: String,
}

pub fn runner_main(config: Config) -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let buf = BufReader::new(stdin);

    let payload: BUUU = serde_json::from_reader(buf)?;

    let manifest_path = cargo::try_locate_manifest().expect("Failed to locate manifest");

    let terminal = Terminal::new();
    let crate_context = ProjectContext::load_from_manifest(&manifest_path);
    let function_registry = FunctionRegistry::new();
    let context = CliContext::new(terminal, &config, crate_context, function_registry);

    local_run_internal(
        payload.backend,
        payload.config,
        payload.overrides,
        payload.function,
        payload.namespace,
        payload.project,
        payload.project_version,
        payload.key,
        context,
    )?;

    Ok(())
}
