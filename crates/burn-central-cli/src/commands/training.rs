use anyhow::Context;
use clap::Parser;
use clap::ValueHint;
use colored::Colorize;

use crate::commands::package::package_sequence;
use crate::execution::{RunKind, execute_experiment_command};
use crate::{
    context::CliContext,
    execution::{BuildCommand, RunCommand, RunParams},
    generation::backend::BackendType,
    logging::BURN_ORANGE,
    print_info,
};
use crate::{print_warn, registry::Flag};

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

fn apply_override(obj: &mut serde_json::Value, key_path: &str, value: serde_json::Value) {
    let mut parts = key_path.split('.').peekable();
    let mut target = obj;

    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            // Last part, set value
            if let serde_json::Value::Object(map) = target {
                map.insert(part.to_string(), value.clone());
            }
        } else {
            target = target
                .as_object_mut()
                .unwrap()
                .entry(part)
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        }
    }
}

fn parse_key_val(s: &str) -> Result<(String, serde_json::Value), String> {
    let (key, value) = s.split_once('=').ok_or("Must be key=value")?;
    let json_value = serde_json::from_str(value)
        .unwrap_or_else(|_| serde_json::Value::String(value.to_string()));
    Ok((key.to_string(), json_value))
}

fn load_config(args: &TrainingArgs) -> serde_json::Value {
    let mut base_json = if let Some(path) = &args.config {
        let text = std::fs::read_to_string(path).expect("failed to read config file");
        serde_json::from_str(&text).expect("failed to parse config file")
    } else {
        serde_json::json!({})
    };

    for (key, val) in &args.overrides {
        apply_override(&mut base_json, key, val.clone());
    }

    serde_json::from_value(base_json).expect("final deserialization failed")
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
    let flags = crate::registry::get_flags();
    print_available_training_functions(&flags);

    let kind = RunKind::Training;
    let function = args.function.clone();
    let namespace = context.get_project_path()?.owner_name;
    let project = context.get_project_path()?.project_name;
    let key = context
        .get_api_key()
        .context("Failed to get API key")?
        .to_owned();
    let backend = args.backend.clone().unwrap_or_default();
    let run_id = format!("{backend}");
    let config = load_config(&args).to_string();

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
                function,
                config,
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

fn print_available_training_functions(flags: &[Flag]) {
    for function in flags.iter().filter(|flag| flag.proc_type == "training") {
        print_info!("{}", format_function_flag(function));
    }
}

#[allow(dead_code)]
fn check_function_registered(function: &str, flags: &[Flag]) -> anyhow::Result<()> {
    let function_flags: Vec<&Flag> = flags
        .iter()
        .filter(|flag| flag.fn_name == function)
        .collect();

    match function_flags.len() {
        0 => Err(anyhow::anyhow!(format!(
            "Function `{}` is not registered as a training function. Annotate a training function with #[burn(training)] to register it.",
            function
        ))),
        1 => Ok(()),
        _ => {
            let function_strings: String = function_flags
                .iter()
                .map(|flag| format_function_flag(flag))
                .collect::<Vec<String>>()
                .join("\n");

            Err(anyhow::anyhow!(format!(
                "Function `{}` is registered multiple times. Please provide the fully qualified function name by writing the entire module path of the function:\n{}",
                function.custom_color(BURN_ORANGE).bold(),
                function_strings
            )))
        }
    }
}

fn format_function_flag(flag: &Flag) -> String {
    format!(
        "  - {}::{} as {}",
        flag.mod_path.bold(),
        flag.fn_name.bold(),
        flag.routine_name.custom_color(BURN_ORANGE).bold()
    )
}
