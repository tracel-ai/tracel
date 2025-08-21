use anyhow::Context;
use burn_central_domain::experiments::config::ModelConfig;
use clap::Parser;
use clap::ValueHint;
use colored::Colorize;

use crate::commands::package::package_sequence;
use crate::execution::{RunKind, execute_experiment_command};
use crate::print_warn;
use crate::{
    context::CliContext,
    execution::{BuildCommand, RunCommand, RunParams},
    generation::backend::BackendType,
    logging::BURN_ORANGE,
    print_info,
};

fn parse_key_val(s: &str) -> Result<(String, serde_json::Value), String> {
    let (key, value) = s.split_once('=').ok_or("Must be key=value")?;
    let json_value = serde_json::from_str(value)
        .unwrap_or_else(|_| serde_json::Value::String(value.to_string()));
    Ok((key.to_string(), json_value))
}

#[derive(Parser, Debug)]
pub struct TrainingArgs {
    /// The training function to run. Annotate a training function with #[burn(training)] to register it.
    function: String,
    /// Backend to use
    #[clap(short = 'b', long = "backend")]
    backend: Option<BackendType>,
    /// Config file path
    #[clap(short = 'c', long = "config")]
    config: Option<String>,
    /// Batch override: e.g. --overrides a.b=3 x.y.z=true
    #[clap(long = "overrides", value_parser = parse_key_val, value_hint = ValueHint::Other, value_delimiter = ' ', num_args = 1..)]
    overrides: Vec<(String, serde_json::Value)>,
    /// Project version
    #[clap(long = "version", help = "The project version.")]
    project_version: Option<String>,
    /// The runner group name
    #[clap(long = "runner", help = "The runner group name.")]
    runner: Option<String>,
}

pub(crate) fn handle_command(args: TrainingArgs, context: CliContext) -> anyhow::Result<()> {
    match (&args.runner, &args.project_version) {
        (Some(_), Some(_)) => Err(anyhow::anyhow!(
            "Remote training is not currently supported."
        )), // remote_run(args, context),
        (None, None) => local_run(args, context),
        (Some(_), None) => Err(anyhow::anyhow!(
            "You must provide the project version to run on the runner with --version argument"
        )),
        (None, Some(_)) => {
            print_warn!(
                "Project version is ignored when executing locally (i.e. no runner is defined with --runner argument"
            );
            local_run(args, context)
        }
    }
}

fn local_run(args: TrainingArgs, context: CliContext) -> anyhow::Result<()> {
    let kind = RunKind::Training;
    let namespace = context.get_project_path()?.owner_name;
    let project = context.get_project_path()?.project_name;
    let key = context
        .get_api_key()
        .context("Failed to get API key")?
        .to_owned();
    let backend = args.backend.clone().unwrap_or_default();
    let run_id = format!("{backend}");
    let config = ModelConfig::load_config(args.config, args.overrides);

    let code_version_digest = package_sequence(&context, false)?;

    let command_to_run: (BuildCommand, RunCommand) = (
        BuildCommand {
            run_id: run_id.clone(),
            backend,
            code_version_digest,
        },
        RunCommand {
            run_id: run_id.clone(),
            run_params: RunParams {
                kind,
                function: args.function.clone(),
                config: config.data.to_string(),
                namespace,
                project,
                key,
            },
        },
    );

    let res = execute_experiment_command(command_to_run.0, command_to_run.1, &context);

    match res {
        Ok(()) => {
            print_info!(
                "Training function `{}` executed successfully.",
                args.function.custom_color(BURN_ORANGE).bold()
            );
        }
        Err(e) => {
            return Err(anyhow::anyhow!(format!(
                "Failed to execute training function `{}`: {}",
                args.function.custom_color(BURN_ORANGE).bold(),
                e
            )));
        }
    }

    Ok(())
}
