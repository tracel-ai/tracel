use clap::{Parser, Subcommand};

use crate::commands::time::format_duration;
use crate::config::Config;
use crate::context::{BurnCentralCliContext, ProjectMetadata};
use crate::{cargo, cli_commands, print_err, print_info};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct CliArgs {
    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
#[command(arg_required_else_help = true)]
pub enum Commands {
    /// Run a training or inference locally or trigger a remote run.
    #[command(subcommand)]
    Run(cli_commands::run::RunLocationType),

    /// Package your project for running on a remote machine.
    Package(cli_commands::package::PackageArgs),
    // todo
    // Ls(),
    // todo
    // Login,
    // todo
    // Logout,
}

fn create_crate_context(config: &Config, args: &CliArgs) -> BurnCentralCliContext {
    let manifest_path =
        cargo::try_locate_manifest().expect("Failed to locate manifest");


    let crate_context = ProjectMetadata::new(&manifest_path);
    BurnCentralCliContext::new(&config, crate_context).init()
}

pub fn cli_main(config: Config) {
    print_info!("Running CLI");
    let time_begin = std::time::Instant::now();
    let args = CliArgs::try_parse();
    if args.is_err() {
        print_err!("{}", args.unwrap_err());
        std::process::exit(1);
    }

    let context = create_crate_context(&config, &args.as_ref().unwrap());

    let cli_res = match args.unwrap().command {
        Commands::Run(run_args) => cli_commands::run::handle_command(run_args, context),
        Commands::Package(package_args) => {
            cli_commands::package::handle_command(package_args, context)
        }
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
