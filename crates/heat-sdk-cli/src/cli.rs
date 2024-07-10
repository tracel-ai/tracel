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
    /// Determines whether experiments sohuld be run in parallel or sequentially. Run in parallel if true.
    #[clap(long = "parallel", default_value = "false")]
    parallel: bool,
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

#[derive(Debug, Clone, ValueEnum, Display, Hash, PartialEq, Eq)]
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

fn execute_experiment_command(
    build_command: BuildCommand,
    run_command: RunCommand,
    project_dir: &str,
    parallel: bool,
) -> Result<(), String> {
    execute_build_command(build_command, project_dir, parallel)?;
    execute_run_command(run_command)?;

    Ok(())
}

fn execute_build_command(
    mut build_command: BuildCommand,
    project_dir: &str,
    parallel: bool,
) -> Result<(), String> {
    print_info!(
        "Building experiment project with command: {:?}",
        build_command
    );
    crate::crate_gen::create_crate(
        build_command
            .burn_features
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<&str>>(),
    );
    generate_metadata_file(project_dir, &build_command.backend);

    let build_status = build_command.command.status();
    match build_status {
        Err(e) => {
            return Err(format!("Failed to build experiment project: {:?}", e));
        }
        Ok(status) if !status.success() => {
            return Err(format!(
                "Failed to build experiment project: {:?}",
                build_command
            ));
        }
        _ => {
            print_info!("Project built successfully.");
        }
    }

    const EXE: &str = std::env::consts::EXE_SUFFIX;

    let src_exe_path = format!(
        "{}/.heat/crates/heat-sdk-cli/target/release/generated_heat_crate{}",
        &project_dir, EXE
    );
    let dest_exe_path = format!(
        "{}/.heat/bin/generated_heat_crate_{}{}",
        &project_dir, build_command.run_id, EXE
    );

    std::fs::create_dir_all(format!("{}/.heat/bin", &project_dir))
        .expect("Failed to create bin directory");
    if let Err(e) = std::fs::copy(src_exe_path, dest_exe_path) {
        if !parallel {
            return Err(format!("Failed to copy executable: {:?}", e));
        }
    }

    Ok(())
}

fn execute_run_command(mut run_command: RunCommand) -> Result<(), String> {
    print_info!("Running experiment with command: {:?}", run_command);
    let run_status = run_command.command.status();
    match run_status {
        Err(e) => {
            return Err(format!("Error running experiment command: {:?}", e));
        }
        Ok(status) if !status.success() => {
            return Err(format!("Failed to run experiment: {:?}", run_command));
        }
        _ => {
            print_info!("Experiment ran successfully.");
        }
    }

    Ok(())
}

fn execute_sequentially(
    commands: Vec<(BuildCommand, RunCommand)>,
    project_dir: &str,
) -> Result<(), String> {
    for cmd in commands {
        execute_experiment_command(cmd.0, cmd.1, project_dir, false)?
    }

    Ok(())
}

fn execute_parallel_build_all_then_run(
    commands: Vec<(BuildCommand, RunCommand)>,
    project_dir: &str,
) -> Result<(), String> {
    let (build_commands, run_commands): (Vec<BuildCommand>, Vec<RunCommand>) =
        commands.into_iter().unzip();

    // Execute all build commands in parallel
    let mut handles = vec![];
    for build_command in build_commands {
        let inner_project_dir = project_dir.to_string();

        let handle = std::thread::spawn(move || {
            execute_build_command(build_command, &inner_project_dir, true)
                .expect("Should be able to build experiment.");
        });
        handles.push(handle);
    }

    for handle in handles {
        match handle.join() {
            Ok(_) => {}
            Err(e) => {
                return Err(format!("Failed to join thread: {:?}", e));
            }
        }
    }

    // Execute all run commands in parallel
    let mut handles = vec![];
    for run_command in run_commands {
        let handle = std::thread::spawn(move || {
            execute_run_command(run_command).expect("Should be able to build and run experiment.");
        });
        handles.push(handle);
    }
    for handle in handles {
        match handle.join() {
            Ok(_) => {}
            Err(e) => {
                return Err(format!("Failed to join thread: {:?}", e));
            }
        }
    }

    Ok(())
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
    run_id: String,
}

pub fn cli_main() {
    print_info!("Running CLI.");
    let time_begin = std::time::Instant::now();
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
                let run_id = format!("{}", backend);

                let mut build_cmd = StdCommand::new("cargo");
                build_cmd
                    .arg("build")
                    .current_dir(&project_dir)
                    .env("HEAT_PROJECT_DIR", &project_dir)
                    .args(["--manifest-path", ".heat/crates/heat-sdk-cli/Cargo.toml"])
                    .args(["--message-format", "short"])
                    .arg("--release");

                const EXE: &str = std::env::consts::EXE_SUFFIX;
                let dest_exe_path = format!(
                    "{}/.heat/bin/generated_heat_crate_{}{}",
                    &project_dir, run_id, EXE
                );

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
                        run_id,
                    },
                    RunCommand { command: run_cmd },
                ));
            }
        }
    }

    let res = if run_args.parallel {
        execute_parallel_build_all_then_run(commands_to_run, &project_dir)
    } else {
        execute_sequentially(commands_to_run, &project_dir)
    };

    match res {
        Ok(()) => {
            print_info!("All experiments have run succesfully!.");
            print_info!(
                "Experiments took {} seconds to run!",
                time_begin.elapsed().as_secs_f64()
            );
        }
        Err(e) => {
            print_err!("An error has occured while running experiments: {}", e);
            std::process::exit(1);
        }
    }
}
