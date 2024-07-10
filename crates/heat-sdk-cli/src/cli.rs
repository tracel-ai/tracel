use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use std::{path::PathBuf, process::Command as StdCommand};
use strum::Display;

use crate::logging::BURN_ORANGE;
use crate::{print_err, print_info};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// {remote|local}
    /// [--backend={wgpu|cuda|candle|tch}]
    #[command(subcommand)]
    Run(RunLocationType),
    // #[command(subcommand)]
    // Ls(LsSubcommand),

    // /// todo
    // Login,
    // /// todo
    // Logout,
}

pub struct LsSubcommand {}

#[derive(Parser, Debug)]
enum RunLocationType {
    #[command(subcommand)]
    Local(LocalRunSubcommand),
    // /// todo
    // #[command(subcommand)]
    // Remote(RemoteRunSubcommand),
}

#[derive(Parser, Debug)]
enum LocalRunSubcommand {
    Training(LocalTrainingRunArgs),
    // Inference(LocalInferenceRunArgs),
}

#[derive(Parser, Debug)]
struct LocalTrainingRunArgs {
    /// The training functions to run
    #[clap(short = 'f', long="functions", value_delimiter = ',', num_args = 1.., required = true)]
    functions: Vec<String>,
    /// Backend to use
    #[clap(short = 'b', long = "backends", value_delimiter = ' ', num_args = 1.., required = true)]
    backends: Vec<BackendValue>,
    /// Config file path
    #[clap(short = 'c', long = "configs", value_delimiter = ' ', num_args = 1.., required = true)]
    configs: Vec<String>,
    /// The project ID
    // todo: support project name and creating a project if it doesn't exist
    #[clap(short = 'p', long = "project", required = true)]
    project: String,
    /// The API key
    #[clap(short = 'k', long = "key", required = true)]
    key: String,
}

#[derive(Parser, Debug)]
struct LocalInferenceRunArgs {
    function: String,
    model_path: PathBuf,
    /// Backend to use
    #[clap(short = 'b', long = "backends", value_delimiter = ' ', num_args = 1.., required = true)]
    backends: Vec<BackendValue>,
    /// The project ID
    // todo: support project name and creating a project if it doesn't exist
    #[clap(short = 'p', long = "project", required = true)]
    project: String,
    /// The API key
    #[clap(short = 'k', long = "key", required = true)]
    key: String,
}

#[derive(Parser, Debug)]
enum RemoteRunSubcommand {
    /// todo
    Training(RemoteTrainingRunArgs),
    /// todo
    Inference(RemoteInferenceRunArgs),
}

#[derive(Parser, Debug)]
struct RemoteTrainingRunArgs {
    //todo
}
#[derive(Parser, Debug)]
struct RemoteInferenceRunArgs {
    //todo
}

#[derive(Debug, Clone, ValueEnum, Display)]
#[strum(serialize_all = "snake_case")]
enum BackendValue {
    Wgpu,
    Tch,
    Ndarray,
}

fn generate_metadata_file(project_dir: &str, backend: &BackendValue) {
    let metadata_file_path = format!(
        "{}/.heat/crates/heat-sdk-cli/run_metadata.toml",
        project_dir
    );

    let mut metadata_toml = toml_edit::DocumentMut::new();
    let mut options = toml_edit::Table::new();
    options["backend"] = toml_edit::value(backend.to_string());

    metadata_toml["options"] = toml_edit::Item::Table(options);

    // Check if the metadata file exists and if it's different from the current metadata
    let should_write = match std::fs::read(&metadata_file_path) {
        Ok(ref content) => content != metadata_toml.to_string().as_bytes(),
        Err(_) => true,
    };

    if should_write {
        if let Err(e) = std::fs::write(metadata_file_path, metadata_toml.to_string()) {
            eprintln!("Failed to write bin file: {}", e);
        }
    }
}

#[derive(Debug)]
pub struct RunCommand {
    command: StdCommand,
}

#[derive(Debug)]
pub struct BuildCommand {
    command: StdCommand,
    backend: BackendValue,
    burn_features: Vec<String>,
}

pub fn cli_main() {
    print_info!("Running CLI.");
    let args = Args::try_parse();
    if args.is_err() {
        print_err!("{}", args.unwrap_err());
        std::process::exit(1);
    }

    let run_args = match args.unwrap().command {
        Commands::Run(RunLocationType::Local(LocalRunSubcommand::Training(run_args))) => run_args,
    };

    let project_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");

    // Check that all passed functions exist
    let flags = crate::registry::get_flags();
    for function in &run_args.functions {
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

    for backend in &run_args.backends {
        for config_path in &run_args.configs {
            for function in &run_args.functions {
                let burn_features: Vec<String> = vec![backend.to_string()];

                let mut build_cmd = StdCommand::new("cargo");
                build_cmd
                    .arg("build")
                    .current_dir(&project_dir)
                    .env("HEAT_PROJECT_DIR", &project_dir)
                    .args(["--manifest-path", ".heat/crates/heat-sdk-cli/Cargo.toml"])
                    .args(["--message-format", "short"])
                    .arg("--release");

                const EXE: &str = std::env::consts::EXE_SUFFIX;

                let src_exe_path = format!("{}/.heat/crates/heat-sdk-cli/target/release/generated_heat_crate{}", &project_dir, EXE);
                let dest_exe_path = format!("{}/.heat/bin/generated_heat_crate{}", &project_dir, EXE);
                
                std::fs::create_dir_all(format!("{}/.heat/bin", &project_dir)).expect("Failed to create bin directory");
                std::fs::copy(&src_exe_path, &dest_exe_path).expect("Failed to copy executable");

                let mut run_cmd = StdCommand::new(dest_exe_path);
                run_cmd
                    .current_dir(&project_dir)
                    .env("HEAT_PROJECT_DIR", &project_dir)
                    .args(["--project", &run_args.project])
                    .args(["--key", &run_args.key])
                    .args(["train", function, config_path]);

                commands_to_run.push((
                    BuildCommand {
                        command: build_cmd,
                        backend: backend.clone(),
                        burn_features: burn_features.clone(),
                    },
                    RunCommand { command: run_cmd },
                ));
            }
        }
    }

    for mut cmd in commands_to_run {
        print_info!("Building experiment project with command: {:?}", cmd.0);
        crate::crate_gen::create_crate(
            cmd.0
                .burn_features
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<&str>>(),
        );
        generate_metadata_file(&project_dir, &cmd.0.backend);

        let build_status = cmd.0.command.status();
        match build_status {
            Err(e) => {
                print_err!("Failed to build experiment project: {:?}", e);
                std::process::exit(1);
            }
            Ok(status) if !status.success() => {
                print_err!("Failed to build experiment project: {:?}", cmd);
                std::process::exit(1);
            }
            _ => {
                print_info!("Project built successfully.");
            }
        }

        print_info!("Running experiment with command: {:?}", cmd.1);
        let run_status = cmd.1.command.status();
        match run_status {
            Err(e) => {
                print_err!("Failed to run experiment: {:?}", e);
                std::process::exit(1);
            }
            Ok(status) if !status.success() => {
                print_err!("Failed to run experiment: {:?}", cmd);
                std::process::exit(1);
            }
            _ => {
                print_info!("Experiment ran successfully.");
            }
        }
    }

    print_info!("Successfully ran all experiments. Exiting.");
}
