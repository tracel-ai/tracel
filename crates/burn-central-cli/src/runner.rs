use crate::{
    commands::training::local_run_internal,
    config::Config,
    context::CliContext,
    entity::projects::ProjectContext,
    generation::backend::BackendType,
    tools::{cargo, functions_registry::FunctionRegistry, terminal::Terminal},
};

#[derive(serde::Deserialize, serde::Serialize)]
pub struct RunnerTrainingArgs {
    /// The training function to run. Annotate a training function with #[burn(training)] to register it.
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

pub fn runner_main(config: Config) {
    let manifest_path = cargo::try_locate_manifest().expect("Should be able to locate manifest.");

    let terminal = Terminal::new();
    let crate_context = ProjectContext::load_from_manifest(&manifest_path);
    let function_registry = FunctionRegistry::new();
    let context = CliContext::new(terminal, &config, crate_context, function_registry);

    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        panic!("Expected exactly one argument");
    }
    let payload: RunnerTrainingArgs =
        serde_json::from_str(&args[1]).expect("Should be able to parse payload");

    let backend = payload.backend.unwrap_or_default();

    local_run_internal(
        backend,
        payload.config,
        payload.overrides,
        payload.function,
        payload.namespace,
        payload.project,
        payload.project_version,
        payload.key,
        &context,
    )
    .inspect_err(|err| {
        context
            .terminal()
            .print(&format!("Should be able to run training function: {err}"));
    })
    .unwrap();
}
