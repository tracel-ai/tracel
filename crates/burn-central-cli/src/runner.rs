use std::{
    io::{BufRead, BufReader},
    time::{Duration, Instant},
};

use crate::{
    commands::training::local_run_internal,
    config::Config,
    context::CliContext,
    entity::projects::ProjectContext,
    generation::backend::BackendType,
    tools::{cargo, functions_registry::FunctionRegistry, terminal::Terminal},
};

const INPUT_TIMEOUT: Duration = Duration::from_secs(30);

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

    let start_time = Instant::now();
    let stdin = std::io::stdin();
    let mut buf = BufReader::new(stdin);
    let mut accumulated_input = String::new();

    let payload = loop {
        if start_time.elapsed() > INPUT_TIMEOUT {
            context.terminal().print("Timeout waiting for valid input");
            std::process::exit(1);
        }

        let mut line = String::new();
        match buf.read_line(&mut line) {
            Ok(0) => {
                context.terminal().print("EOF reached without valid input");
                std::process::exit(1);
            }
            Ok(_) => {
                accumulated_input.push_str(&line);

                // Try to parse the accumulated input
                match serde_json::from_str::<RunnerTrainingArgs>(&accumulated_input) {
                    Ok(payload) => break payload,
                    Err(_) => {
                        // Continue reading more lines
                        continue;
                    }
                }
            }
            Err(err) => {
                context
                    .terminal()
                    .print(&format!("Error reading input: {err}"));
                std::process::exit(1);
            }
        }
    };

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
