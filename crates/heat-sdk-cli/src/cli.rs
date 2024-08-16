use clap::{Parser, Subcommand};

use crate::commands::time::format_duration;
use crate::context::HeatCliContext;
use crate::{cli_commands, print_err, print_info};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct CliArgs {
    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
#[command(arg_required_else_help = true)]
pub enum Commands {
    /// {local|remote} : Run a training or inference locally or trigger a remote run.
    #[command(subcommand)]
    Run(cli_commands::run::RunLocationType),
    // todo
    // Ls(),
    // todo
    // Login,
    // todo
    // Logout,
}

pub fn cli_main() {
    print_info!("Running CLI.");
    let time_begin = std::time::Instant::now();
    let args = CliArgs::try_parse();
    if args.is_err() {
        print_err!("{}", args.unwrap_err());
        std::process::exit(1);
    }

    let user_project_name = std::env::var("CARGO_PKG_NAME").expect("CARGO_PKG_NAME not set");
    let user_crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");

    let context = HeatCliContext::new(user_project_name, user_crate_dir.into()).init();

    let cli_res = match args.unwrap().command {
        Commands::Run(run_args) => cli_commands::run::handle_command(run_args, context),
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
