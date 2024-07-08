use clap::{Parser, Subcommand, ValueEnum};
use std::process::Command as StdCommand;
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
    Run(RunType),
    /// #todo
    Login,
    /// #todo
    Logout,
}

#[derive(Parser, Debug)]
enum RunType {
    Local(LocalRunArgs),
    Remote(RemoteRunArgs),
}

#[derive(Parser, Debug)]
struct LocalRunArgs {
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
struct RemoteRunArgs {
    // #todo
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
}

pub fn cli_main() {
    println!("Running CLI.");
    let args: Args = Args::parse();
    let run_args = match args.command {
        Commands::Run(RunType::Local(run_args)) => run_args,
        _ => unimplemented!(),
    };

    let mut commands_to_run: Vec<StdCommand> = Vec::new();

    for backend in &run_args.backends {
        for config_path in &run_args.configs {
            let mut feature_flags: Vec<String> = Vec::new();

            feature_flags.push(backend.into_heat_feature_flag());

            let mut cmd = StdCommand::new("cargo");
            cmd.arg("run")
                .arg("--release")
                .args(vec!["--bin", "guide-test"])
                .args(vec!["--features", &feature_flags.join(",")])
                .arg("--")
                .args(vec!["--config", &config_path])
                .args(vec!["--project", &run_args.project])
                .args(vec!["--key", &run_args.key]);

            commands_to_run.push(cmd);
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
