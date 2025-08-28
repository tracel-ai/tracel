use crate::{
    commands::training::local_run_internal,
    config::Config,
    context::CliContext,
    entity::projects::ProjectContext,
    generation::backend::BackendType,
    tools::{cargo, functions_registry::FunctionRegistry, terminal::Terminal},
};

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ComputeProviderTrainingArgs {
    /// The training function to run.
    pub function: String,
    /// Backend to use
    pub backend: Option<BackendType>,
    /// Config file path
    pub config: Option<String>,
    /// Batch override: e.g. --overrides a.b=3 x.y.z=true
    pub overrides: Vec<(String, serde_json::Value)>,
    /// Project version
    pub project_version: String,
    pub namespace: String,
    pub project: String,
    pub key: String,
}

pub fn compute_provider_main(config: Config) {
    let manifest_path = cargo::try_locate_manifest().expect("Should be able to locate manifest.");

    let terminal = Terminal::new();
    let crate_context = ProjectContext::load_from_manifest(&manifest_path);
    let function_registry = FunctionRegistry::new();
    let context = CliContext::new(terminal, &config, crate_context, function_registry);

    let args = get_args();

    let backend = args.backend.unwrap_or_default();

    local_run_internal(
        backend,
        args.config,
        args.overrides,
        args.function,
        args.namespace,
        args.project,
        args.project_version,
        args.key,
        &context,
    )
    .inspect_err(|err| {
        context
            .terminal()
            .print(&format!("Should be able to run training function: {err}"));
    })
    .unwrap();
}

fn get_args() -> ComputeProviderTrainingArgs {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        panic!("Expected exactly one argument");
    }

    serde_json::from_str(&args[1]).expect("Should be able to parse payload")
}
