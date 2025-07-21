use anyhow::Context as _;
use clap::{Parser, Subcommand};

use crate::commands::time::format_duration;
use crate::config::Config;
use crate::context::{CliContext, ProjectContext};
use crate::terminal::Terminal;
use crate::{cargo, cli_commands, print_err, print_info};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run a training or inference locally or trigger a remote run.
    #[command(subcommand)]
    Run(cli_commands::run::RunLocationType),

    /// Package your project for running on a remote machine.
    Package(cli_commands::package::PackageArgs),
    /// Log in to the Burn Central server.
    Login(cli_commands::login::LoginArgs),
    /// Initialize a new project or reinitialize an existing one.
    Init(cli_commands::init::InitArgs),
}

pub fn cli_main(config: Config) {
    print_info!("Running CLI");
    let time_begin = std::time::Instant::now();
    let args = CliArgs::parse();

    let manifest_path = cargo::try_locate_manifest().expect("Failed to locate manifest");

    let terminal = Terminal::new();
    let crate_context = ProjectContext::load_from_manifest(&manifest_path);
    let context = CliContext::new(terminal, &config, crate_context).init();

    let cli_res = match args.command {
        Some(command) => handle_command(command, context),
        None => default_command(context),
    };

    match cli_res {
        Ok(_) => {
            print_info!("CLI command executed successfully.");
        }
        Err(e) => {
            print_err!("Error executing CLI command: {}", e);
        }
    }

    let duration = time_begin.elapsed();
    print_info!(
        "\x1B[32;1mTime elapsed for the current execution: {}\x1B[0m",
        format_duration(&duration)
    );
}

fn handle_command(command: Commands, mut context: CliContext) -> anyhow::Result<()> {
    if matches!(command, Commands::Run(..) | Commands::Package(..)) {
        if let Err(e) = context.load_project() {
            return Err(anyhow::anyhow!("Failed to load project metadata: {}.", e));
        }
    }

    match command {
        Commands::Run(run_args) => cli_commands::run::handle_command(run_args, context),
        Commands::Package(package_args) => {
            cli_commands::package::handle_command(package_args, context)
        }
        Commands::Login(login_args) => cli_commands::login::handle_command(login_args, context),
        Commands::Init(init_args) => cli_commands::init::handle_command(init_args, context),
    }
}

fn default_command(mut context: CliContext) -> anyhow::Result<()> {
    let project_loaded = context.load_project().is_ok();

    let client = cli_commands::login::get_client_and_login_if_needed(&mut context)?;

    if !project_loaded {
        print_info!("No project loaded. Running initialization sequence.");
        cli_commands::init::prompt_init(&context, &client)?;

        cli_commands::package::handle_command(cli_commands::package::PackageArgs {}, context)?;
    } else {
        print_info!("No command provided. Please specify a command to run.");
    }

    Ok(())
}
