use clap::Parser;
use colored::Colorize;

use crate::{
    commands::{
        execute_parallel_build_all_then_run, execute_sequentially, BuildCommand, RunCommand,
    },
    crate_gen::backend::BackendType,
    logging::BURN_ORANGE,
    print_err, print_info,
};
use std::process::Command as StdCommand;

#[derive(Parser, Debug)]
pub struct LocalTrainingRunArgs {
    /// The training functions to run
    #[clap(short = 'f', long="functions", value_delimiter = ' ', num_args = 1.., required = true, help = "<required> The training functions to run. Annotate a training function with #[heat(training)] to register it.")]
    functions: Vec<String>,
    /// Backend to use
    #[clap(short = 'b', long = "backends", value_delimiter = ' ', num_args = 1.., required = true, help = "<required> Backends to use for training.")]
    backends: Vec<BackendType>,
    /// Config files paths
    #[clap(short = 'c', long = "configs", value_delimiter = ' ', num_args = 1.., required = true, help = "<required> Config files paths.")]
    configs: Vec<String>,
    /// The Heat project ID
    // todo: support project name and creating a project if it doesn't exist
    #[clap(
        short = 'p',
        long = "project",
        required = true,
        help = "<required> The Heat project ID."
    )]
    project: String,
    /// The Heat API key
    #[clap(
        short = 'k',
        long = "key",
        required = true,
        help = "<required> The Heat API key."
    )]
    key: String,
    /// Determines whether experiments sohuld be run in parallel or sequentially. Run in parallel if true.
    #[clap(long = "parallel", default_value = "false")]
    parallel: bool,
}

pub(crate) fn handle_command(args: LocalTrainingRunArgs) -> anyhow::Result<()> {
    let project_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");

    // print all functions that are registered as training functions
    let flags = crate::registry::get_flags();
    let training_functions = flags
        .iter()
        .filter(|flag| flag.proc_type == "training")
        .map(|flag| {
            format!(
                "  {} {}::{}",
                "-".custom_color(BURN_ORANGE),
                flag.mod_path.bold(),
                flag.fn_name.bold()
            )
        })
        .collect::<Vec<String>>();
    print_info!("Registered training functions:");
    for function in training_functions {
        print_info!("{}", function);
    }

    // Check that all passed functions exist
    let flags = crate::registry::get_flags();
    for function in &args.functions {
        let function_flags = flags
            .iter()
            .filter(|flag| flag.fn_name == function)
            .collect::<Vec<&crate::registry::Flag>>();
        if function_flags.is_empty() {
            print_err!("Function `{}` is not registered as a training function. Annotate a training function with #[heat(training)] to register it.", function);
            std::process::exit(1);
        } else if function_flags.len() > 1 {
            let function_strings = function_flags
                .iter()
                .map(|flag| {
                    format!(
                        "  {} {}::{}",
                        "-".custom_color(BURN_ORANGE),
                        flag.mod_path.bold(),
                        flag.fn_name.bold()
                    )
                })
                .collect::<Vec<String>>();
            print_err!("Function `{}` is registered multiple times. Please write the entire module path of the desired function:\n{}", function.custom_color(BURN_ORANGE).bold(), function_strings.join("\n"));
            std::process::exit(1);
        }
    }

    let mut commands_to_run: Vec<(BuildCommand, RunCommand)> = Vec::new();

    for backend in &args.backends {
        for config_path in &args.configs {
            for function in &args.functions {
                let burn_features: Vec<String> = vec![backend.to_string()];
                let run_id = format!("{}", backend);

                let mut build_cmd = StdCommand::new("cargo");
                build_cmd
                    .arg("build")
                    .arg("--release")
                    .arg("--no-default-features")
                    .current_dir(&project_dir)
                    .env("HEAT_PROJECT_DIR", &project_dir)
                    .args([
                        "--manifest-path",
                        ".heat/crates/generated-heat-sdk-crate/Cargo.toml",
                    ])
                    .args(["--message-format", "short"]);

                const EXE: &str = std::env::consts::EXE_SUFFIX;
                let dest_exe_path = format!(
                    "{}/.heat/bin/generated-heat-sdk-crate-{}{}",
                    &project_dir, run_id, EXE
                );

                let mut run_cmd = StdCommand::new(dest_exe_path);
                run_cmd
                    .current_dir(&project_dir)
                    .env("HEAT_PROJECT_DIR", &project_dir)
                    .args(["--project", &args.project])
                    .args(["--key", &args.key])
                    .args(["train", function, config_path]);

                commands_to_run.push((
                    BuildCommand {
                        command: build_cmd,
                        backend: backend.clone(),
                        burn_features: burn_features.clone(),
                        run_id,
                    },
                    RunCommand { command: run_cmd },
                ));
            }
        }
    }

    let res = if args.parallel {
        execute_parallel_build_all_then_run(commands_to_run, &project_dir)
    } else {
        execute_sequentially(commands_to_run, &project_dir)
    };

    match res {
        Ok(()) => {
            print_info!("All experiments have run succesfully!.");
        }
        Err(e) => {
            print_err!("An error has occured while running experiments: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}
