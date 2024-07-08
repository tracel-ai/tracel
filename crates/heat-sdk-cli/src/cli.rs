use clap::{Parser, Subcommand, ValueEnum};
use std::{path::PathBuf, process::Command as StdCommand};
use strum::Display;

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
    /// #todo
    Login,
    /// #todo
    Logout,
}

#[derive(Parser, Debug)]
enum RunLocationType {
    #[command(subcommand)]
    Local(LocalRunSubcommand),
    #[command(subcommand)]
    Remote(RemoteRunSubcommand),
}

#[derive(Parser, Debug)]
enum LocalRunSubcommand {
    Training(LocalTrainingRunArgs),
    Inference(LocalInferenceRunArgs),
}

#[derive(Parser, Debug)]
struct LocalTrainingRunArgs {
    /// The training functions to run
    #[clap(short = 'f', long="function", value_delimiter = ',', num_args = 1.., required = true)]
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

impl BackendValue {
    pub fn into_heat_feature_flag(&self) -> String {
        match self {
            BackendValue::Wgpu => "heat-macros/wgpu".to_string(),
            BackendValue::Tch => "heat-macros/tch".to_string(),
            BackendValue::Ndarray => "heat-macros/ndarray".to_string(),
        }
    }

    pub fn into_burn_feature_flag(&self) -> String {
        match self {
            BackendValue::Wgpu => "burn/wgpu".to_string(),
            BackendValue::Tch => "burn/tch".to_string(),
            BackendValue::Ndarray => "burn/ndarray".to_string(),
        }
    }
}

pub fn cli_main() {
    println!("Running CLI.");
    let args: Args = Args::parse();
    println!("Args: {:?}", args);
    let run_args = match args.command {
        Commands::Run(RunLocationType::Local(LocalRunSubcommand::Training(run_args))) => run_args,
        _ => unimplemented!(),
    };

    let project_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");

    let mut commands_to_run: Vec<StdCommand> = Vec::new();

    for function in &run_args.functions {
        for backend in &run_args.backends {
            for config_path in &run_args.configs {
                let mut feature_flags: Vec<String> = Vec::new();
                let mut burn_features: Vec<String> = Vec::new();

                feature_flags.push(backend.into_heat_feature_flag());

                burn_features.push(backend.into_burn_feature_flag());

                crate::crate_gen::create_crate(
                    burn_features
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<&str>>(),
                );

                let mut cmd = StdCommand::new("cargo");
                cmd.arg("run")
                    .current_dir(&project_dir)
                    .env("HEAT_PROJECT_DIR", &project_dir)
                    .args(vec![
                        "--manifest-path",
                        ".heat/crates/heat-sdk-cli/Cargo.toml",
                    ])
                    .arg("--release")
                    .args(vec!["--features", &feature_flags.join(",")])
                    .arg("--")
                    .args(vec!["--training", &function])
                    .args(vec!["--config", &config_path])
                    .args(vec!["--project", &run_args.project])
                    .args(vec!["--key", &run_args.key]);

                commands_to_run.push(cmd);
            }
        }
    }

    for mut cmd in commands_to_run {
        println!("Running command: {:?}", cmd);

        let status = cmd.status().expect("Failed to execute command");
        if !status.success() {
            panic!("Command failed: {:?}", cmd);
        }
    }
}
