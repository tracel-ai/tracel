use crate::entity::projects::ProjectContext;
use clap::{Parser, Subcommand};

use crate::commands::default_command;
use crate::config::Config;
use crate::context::CliContext;
use crate::tools::cargo;
use crate::tools::functions_registry::FunctionRegistry;
use crate::tools::terminal::Terminal;
use crate::{commands, print_err};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[arg(long, action = clap::ArgAction::SetTrue, hide = true)]
    pub dev: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run a training or inference locally or trigger a remote run.
    Train(commands::training::TrainingArgs),

    /// Package your project for running on a remote machine.
    Package(commands::package::PackageArgs),
    /// Log in to the Burn Central server.
    Login(commands::login::LoginArgs),
    /// Initialize a new project or reinitialize an existing one.
    Init(commands::init::InitArgs),
    /// Unlink the burn central project from this repository.
    Unlink,
    /// Display current user information.
    Me,
    /// Display current project information.
    Project,
}

pub fn cli_main() {
    let args = CliArgs::parse();

    let manifest_path = cargo::try_locate_manifest().expect("Failed to locate manifest");

    let config = Config {
        api_endpoint: if args.dev {
            "http://localhost:9001/".to_string()
        } else {
            "https://heat.tracel.ai/api/".to_string()
        },
    };

    let terminal = Terminal::new();
    let crate_context = ProjectContext::load_from_manifest(&manifest_path);
    let function_registry = FunctionRegistry::new();
    let context =
        CliContext::new(terminal.clone(), &config, crate_context, function_registry).init();

    let cli_res = match args.command {
        Some(command) => handle_command(command, context),
        None => default_command(context),
    };

    if let Err(e) = cli_res {
        terminal.cancel_finalize(&format!("{e}"));
    }
}

fn handle_command(command: Commands, mut context: CliContext) -> anyhow::Result<()> {
    if matches!(command, Commands::Train(..) | Commands::Package(..)) {
        if let Err(e) = context.load_project() {
            return Err(anyhow::anyhow!("Failed to load project metadata: {}.", e));
        }
    }

    match command {
        Commands::Train(run_args) => commands::training::handle_command(run_args, context),
        Commands::Package(package_args) => commands::package::handle_command(package_args, context),
        Commands::Login(login_args) => commands::login::handle_command(login_args, context),
        Commands::Init(init_args) => commands::init::handle_command(init_args, context),
        Commands::Unlink => Ok(commands::unlink::handle_command(context)),
        Commands::Me => Ok(commands::me::handle_command(context)),
        Commands::Project => Ok(commands::project::handle_command(context)),
    }
}
