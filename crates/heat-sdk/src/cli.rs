use clap::{Parser, Subcommand, ValueEnum};
use std::process::Command as StdCommand;
use strum::Display;

use crate::{Flag, Plugin};

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
    Local(RunArgs),
    Remote(RunArgs),
}

#[derive(Parser, Debug)]
struct RunArgs {
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

#[derive(Debug, Clone, ValueEnum, Display)]
enum BackendValue {
    #[strum(to_string = "wgpu")]
    Wgpu,
    #[strum(to_string = "tch")]
    Tch,
    #[strum(to_string = "ndarray")]
    Ndarray,
}

pub fn cli_main() {
    println!("Running CLI.");
    let args: Args = Args::parse();
    let backends = match args.command {
        Commands::Run(ref run_type) => match run_type {
            RunType::Local(run_args) => &run_args.backends,
            RunType::Remote(run_args) => &run_args.backends,
        },
        _ => unimplemented!(),
    };
    let config_paths = match args.command {
        Commands::Run(ref run_type) => match run_type {
            RunType::Local(run_args) => &run_args.configs,
            RunType::Remote(run_args) => &run_args.configs,
        },
        _ => unimplemented!(),
    };
    let project = match args.command {
        Commands::Run(ref run_type) => match run_type {
            RunType::Local(run_args) => &run_args.project,
            RunType::Remote(run_args) => &run_args.project,
        },
        _ => unimplemented!(),
    };
    let key = match args.command {
        Commands::Run(ref run_type) => match run_type {
            RunType::Local(run_args) => &run_args.key,
            RunType::Remote(run_args) => &run_args.key,
        },
        _ => unimplemented!(),
    };

    let mut commands_to_run: Vec<StdCommand> = Vec::new();

    for backend in backends {
        for config_path in config_paths {
            let mut feature_flags: Vec<String> = Vec::new();
            match backend {
                BackendValue::Wgpu => {
                    feature_flags.push("heat-macros/wgpu".to_string());
                }
                BackendValue::Tch => {
                    feature_flags.push("heat-macros/tch".to_string());
                }
                BackendValue::Ndarray => {
                    feature_flags.push("heat-macros/ndarray".to_string());
                }
            }
            let mut cmd = StdCommand::new("cargo");
            cmd.arg("run")
                .arg("--release")
                .args(vec!["--bin", "guide-test"])
                .args(vec!["--features", &feature_flags.join(",")])
                .arg("--")
                .args(vec!["--config", &config_path])
                .args(vec!["--project", &project])
                .args(vec!["--key", &key]);

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