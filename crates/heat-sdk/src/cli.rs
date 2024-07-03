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
    Local(RunArgs),
    Remote(RunArgs),
}

#[derive(Parser, Debug)]
struct RunArgs {
    /// Backend to use
    #[clap(short = 'b', long = "backend", required = true)]
    backend: BackendValue,
    /// Config file path
    #[clap(short = 'c', long = "configs", value_delimiter = ' ', num_args = 1.., required = true)]
    configs: Vec<String>,
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
    let backend = match args.command {
        Commands::Run(ref run_type) => match run_type {
            RunType::Local(run_args) => &run_args.backend,
            RunType::Remote(run_args) => &run_args.backend,
        },
        _ => unimplemented!(),
    };
    let config_path = match args.command {
        Commands::Run(ref run_type) => match run_type {
            RunType::Local(run_args) => &run_args.configs,
            RunType::Remote(run_args) => &run_args.configs,
        },
        _ => unimplemented!(),
    };

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

    let status = StdCommand::new("cargo")
        .arg("run")
        .arg("--release")
        .args(vec!["--bin", "guide-test"])
        .args(vec!["--features", &feature_flags.join(",")])
        .arg("--")
        .args(vec!["--configs", &config_path.join(" ")])
        .status()
        .expect("Failed to build the project");

    if !status.success() {
        panic!("Failed to build the project");
    }
}
