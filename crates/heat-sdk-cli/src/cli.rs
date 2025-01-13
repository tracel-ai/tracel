use clap::{Parser, Subcommand};

use crate::commands::time::format_duration;
use crate::config::Config;
use crate::context::{HeatCliContext, HeatCliCrateContext, HeatCliGlobalContext, ProjectMetadata};
use crate::{cli_commands, print_err, print_info};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct CliArgs {
    #[clap(subcommand)]
    pub command: Commands,
    /// Include cargo manifest arguments
    #[clap(flatten)]
    pub manifest: clap_cargo::Manifest,
}

#[derive(Subcommand, Debug)] 
#[command(arg_required_else_help = true)]
pub enum Commands {
    /// Run a training or inference locally or trigger a remote run.
    Run(cli_commands::run::RunArgs),

    /// Package your project for running on a remote machine.
    Package(cli_commands::package::PackageArgs),
    // todo
    // Ls(),
    // todo
    Login(cli_commands::login::LoginArgs),
    // todo
    // Logout,
}

fn try_locate_manifest() -> Option<std::path::PathBuf> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = std::process::Command::new(cargo)
        .arg("locate-project")
        .output()
        .expect("Failed to run cargo locate-project");
    let output_str = String::from_utf8(output.stdout).expect("Failed to parse output");
    let parsed_output: serde_json::Value = serde_json::from_str(&output_str).expect("Failed to parse output");

    let manifest_path_str = parsed_output["root"]
        .as_str()
        .expect("Failed to parse output")
        .to_owned();

    let manifest_path = std::path::PathBuf::from(manifest_path_str);
    print_info!("Found manifest at: {}", manifest_path.display());
    Some(manifest_path)
}

fn create_crate_context(config: &Config, args: &CliArgs) -> HeatCliCrateContext {
    let manifest_path = if let Some(path) = &args.manifest.manifest_path {
        path.to_owned()
    } else {
        print_info!("No manifest found. Using default manifest.");
        try_locate_manifest().expect("Failed to locate manifest")
    };


    let crate_context = ProjectMetadata::new(&manifest_path);
    HeatCliContext::new(&config, crate_context).init()
}

pub fn cli_main(config: Config) {
    print_info!("Running CLI");
    let time_begin = std::time::Instant::now();
    let args = CliArgs::try_parse();
    if args.is_err() {
        print_err!("{}", args.unwrap_err());
        std::process::exit(1);
    }
    let args = args.unwrap();

    let mut context = create_crate_context(&config, &args);

    let cli_res = match args.command {
        Commands::Run(run_args) => {
            cli_commands::run::handle_command(run_args, &mut context)
        },
        Commands::Package(package_args) => {
            cli_commands::package::handle_command(package_args, &mut context)
        }
        Commands::Login(login_args) => {
            cli_commands::login::handle_command(login_args, &mut context)
        },
    };

    match cli_res {
        Ok(_) => {
            print_info!("CLI command executed successfully.");
        }
        Err(e) => {
            print_err!("Error executing CLI command: {:?}", e);
        }
    }

    let duration = time_begin.elapsed();
    print_info!(
        "\x1B[32;1mTime elapsed for the current execution: {}\x1B[0m",
        format_duration(&duration)
    );
}
