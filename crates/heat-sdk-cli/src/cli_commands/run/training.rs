use anyhow::Context;
use clap::Parser;
use colored::Colorize;

use crate::registry::Flag;
use crate::util::git::{DefaultGitRepo, GitRepo};
use crate::{
    commands::{execute_sequentially, BuildCommand, RunCommand, RunParams},
    generation::backend::BackendType,
    logging::BURN_ORANGE,
    print_info,
};
use crate::context::HeatCliCrateContext;

#[derive(Parser, Debug)]
pub struct TrainingRunArgs {
    /// The training functions to run
    #[clap(short = 'f', long="functions", value_delimiter = ' ', num_args = 1.., required = true, help = "<required> The training functions to run. Annotate a training function with #[heat(training)] to register it."
    )]
    functions: Vec<String>,
    /// Backend to use
    #[clap(short = 'b', long = "backends", value_delimiter = ' ', num_args = 1.., required = true, help = "<required> Backends to use for training."
    )]
    backends: Vec<BackendType>,
    /// Config files paths
    #[clap(short = 'c', long = "configs", value_delimiter = ' ', num_args = 1.., required = true, help = "<required> Config files paths."
    )]
    configs: Vec<String>,
    /// The Heat project ID
    // todo: support project name and creating a project if it doesn't exist
    #[clap(
        short = 'p',
        long = "project",
        required = true,
        help = "<required> The Heat project ID."
    )]
    project_path: String,
    /// The Heat API key
    #[clap(
        short = 'k',
        long = "key",
        help = "<required> The Heat API key."
    )]
    key: Option<String>,
    /// Project version
    #[clap(short = 't', long = "version", help = "The project version.")]
    project_version: Option<String>,
    /// The runner group name
    #[clap(short = 'r', long = "runner", help = "The runner group name.")]
    runner: Option<String>,
}

pub(crate) fn handle_command(args: TrainingRunArgs, context: &mut HeatCliCrateContext) -> anyhow::Result<()> {
    match (&args.runner, &args.project_version) {
        (Some(_), Some(_)) => remote_run(args, context),
        (None, Some(_)) => checkout_local_run(args, context),
        (None, None) => local_run(args, context),
        _ => Err(anyhow::anyhow!("Both runner and project version must be specified for remote run and none for local run.")),
    }
}

fn remote_run(args: TrainingRunArgs, context: &mut HeatCliCrateContext) -> anyhow::Result<()> {
    let heat_client = context.create_heat_client(
        args.key.clone(),
        &args.project_path,
    )?;

    let project_version = args.project_version.unwrap();
    if !heat_client.check_project_version_exists(&project_version)? {
        return Err(anyhow::anyhow!("Project version `{}` does not exist. Please upload your code using the `package` command then you can run your code remotely with that version.", project_version));
    }

    let api_key = args.key.as_deref().or_else(|| context.get_api_key()).context("API key is required")?;

    heat_client.start_remote_job(
        args.runner.unwrap(),
        &project_version,
        format!(
            "run training --functions {} --backends {} --configs {} --project {} --key {}",
            args.functions.join(" "),
            args.backends
                .into_iter()
                .map(|backend| backend.to_string())
                .collect::<Vec<_>>()
                .join(" "),
            args.configs.join(" "),
            args.project_path,
            api_key,
        ),
    )?;

    Ok(())
}

fn checkout_local_run(args: TrainingRunArgs, context: &mut HeatCliCrateContext) -> anyhow::Result<()> {
    let repo = DefaultGitRepo::new()?.if_not_dirty()?;
    let ver = args.project_version.as_deref().unwrap();
    let checkout_guard = if !repo.is_at_commit(ver)? {
        Some(repo.checkout_commit(ver)?)
    } else {
        None
    };

    let run_res = local_run(args, context)?;

    drop(checkout_guard);
    Ok(run_res)
}

fn local_run(args: TrainingRunArgs, context: &mut HeatCliCrateContext) -> anyhow::Result<()> {
    let flags = crate::registry::get_flags();
    print_available_training_functions(&flags);

    for function in &args.functions {
        check_function_registered(function, &flags)?;
    }

    let mut commands_to_run: Vec<(BuildCommand, RunCommand)> = Vec::new();

    let api_key = args.key.as_deref().or_else(|| context.get_api_key()).context("API key is required")?;

    for backend in &args.backends {
        for config_path in &args.configs {
            for function in &args.functions {
                let run_id = format!("{}", backend);

                commands_to_run.push((
                    BuildCommand {
                        run_id: run_id.clone(),
                        backend: backend.clone(),
                    },
                    RunCommand {
                        run_id,
                        run_params: RunParams::Training {
                            function: function.to_owned(),
                            config_path: config_path.to_owned(),
                            project: args.project_path.clone(),
                            key: api_key.to_string(),
                        },
                    },
                ));
            }
        }
    }

    let res = execute_sequentially(commands_to_run, context);

    match res {
        Ok(()) => {
            print_info!("All experiments have run successfully!.");
        }
        Err(e) => {
            return Err(anyhow::anyhow!(format!(
                "An error has occurred while running experiments: {}",
                e
            )));
        }
    }

    Ok(())
}

fn print_available_training_functions(flags: &Vec<Flag>) {
    for function in flags.iter().filter(|flag| flag.proc_type == "training") {
        print_info!("{}", format_function_flag(function));
    }
}

fn check_function_registered(function: &str, flags: &Vec<Flag>) -> anyhow::Result<()> {
    let function_flags: Vec<&Flag> = flags
        .iter()
        .filter(|flag| flag.fn_name == function)
        .collect();

    match function_flags.len() {
        0 => Err(anyhow::anyhow!(format!("Function `{}` is not registered as a training function. Annotate a training function with #[heat(training)] to register it.", function))),
        1 => Ok(()),
        _ => {
            let function_strings: String = function_flags
                .iter()
                .map(|flag| format_function_flag(flag))
                .collect::<Vec<String>>()
                .join("\n");


            Err(anyhow::anyhow!(format!("Function `{}` is registered multiple times. Please write the entire module path of the desired function:\n{}", function.custom_color(BURN_ORANGE).bold(), function_strings)))
        }
    }
}

fn format_function_flag(flag: &Flag) -> String {
    format!(
        "  {} {}::{}",
        "-".custom_color(BURN_ORANGE),
        flag.mod_path.bold(),
        flag.fn_name.bold()
    )
}
