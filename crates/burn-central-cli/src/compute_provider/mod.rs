use crate::{
    commands::training::local_run_internal,
    config::Config,
    context::CliContext,
    entity::projects::ProjectContext,
    generation::backend::BackendType,
    tools::{cargo, functions_registry::FunctionRegistry, terminal::Terminal},
};

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureType {
    Training,
    Inference,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ProcedureTypeArg {
    pub procedure_type: ProcedureType,
}

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
    pub digest: String,
    pub namespace: String,
    pub project: String,
    pub key: String,
    #[serde(flatten)]
    pub procedure_type: ProcedureTypeArg,
}

pub fn compute_provider_main(config: Config) {
    let manifest_path = cargo::try_locate_manifest().expect("Should be able to locate manifest.");

    let terminal = Terminal {};
    let crate_context = ProjectContext::load_from_manifest(&manifest_path);
    let function_registry = FunctionRegistry::new();
    let context = CliContext::new(terminal, &config, crate_context, function_registry);

    let arg = get_arg();
    match get_procedure_type(&arg) {
        ProcedureType::Training => {
            let args = serde_json::from_str::<ComputeProviderTrainingArgs>(&arg)
                .expect("Should be able to deserialize the arg as ComputeProviderTrainingArgs");

            run_training(args, &context);
        }
        _ => {
            panic!("Only training is supported for now")
        }
    }
}

fn get_arg() -> String {
    std::env::args()
        .nth(1)
        .expect("Expected exactly one argument")
}

fn get_procedure_type(arg: &str) -> ProcedureType {
    let proc_type = serde_json::from_str::<ProcedureTypeArg>(arg)
        .expect("Should be able to deserialize the arg as ProcedureTypeArg");

    proc_type.procedure_type
}

fn run_training(args: ComputeProviderTrainingArgs, context: &CliContext) {
    let backend = args.backend.unwrap_or_default();

    local_run_internal(
        backend,
        args.config,
        args.overrides,
        args.function,
        args.namespace,
        args.project,
        args.digest,
        args.key,
        context,
    )
    .inspect_err(|err| {
        context
            .terminal()
            .print(&format!("Should be able to run training function: {err}"));
    })
    .unwrap();
}
