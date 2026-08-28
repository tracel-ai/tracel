//! A terminal tour of the Tracel console SDK: device-flow login, session storage, and browsing
//! organizations, projects, and models.

mod cli;
mod display;
mod interactive;
mod progress;
mod session;

use clap::Parser;
use tracel::console::Env;

use cli::{AuthCommand, Cli, Command, ModelCommand, OrgCommand, ProjectCommand};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    run(cli.environment, cli.command)
}

fn run(env: Env, command: Command) -> anyhow::Result<()> {
    match command {
        Command::Auth(command) => run_auth(env, command),
        Command::Org(command) => run_org(env, command),
        Command::Project(command) => run_project(env, command),
        Command::Model(command) => run_model(env, command),
    }
}

fn run_auth(env: Env, command: AuthCommand) -> anyhow::Result<()> {
    match command {
        AuthCommand::Login => session::login(env),
        AuthCommand::Logout => session::logout(env),
        AuthCommand::Whoami => {
            let console = session::connect(env)?;
            display::current_user(&console)
        }
    }
}

fn run_org(env: Env, command: OrgCommand) -> anyhow::Result<()> {
    match command {
        OrgCommand::List => {
            let console = session::connect(env)?;
            let orgs = console.organizations()?;
            display::organizations(&orgs)
        }
    }
}

fn run_project(env: Env, command: ProjectCommand) -> anyhow::Result<()> {
    match command {
        ProjectCommand::List { namespace } => {
            let console = session::connect(env)?;
            let namespace = interactive::resolve_namespace(&console, namespace)?;
            let projects = console.projects_of(&namespace)?;
            display::projects(&namespace, &projects)
        }
        ProjectCommand::Show { scope } => {
            let console = session::connect(env)?;
            let (owner, project) = interactive::resolve_project(&console, scope)?;
            let details = console.project(&owner, &project).get()?;
            display::project(&details);
            Ok(())
        }
    }
}

fn run_model(env: Env, command: ModelCommand) -> anyhow::Result<()> {
    match command {
        ModelCommand::List { scope } => {
            let console = session::connect(env)?;
            let (owner, project) = interactive::resolve_project(&console, scope)?;
            display::models(
                &console.project(&owner, &project).models(),
                &owner,
                &project,
            )
        }
        ModelCommand::Show { scope, model } => {
            let console = session::connect(env)?;
            let (owner, project) = interactive::resolve_project(&console, scope)?;
            let models = console.project(&owner, &project).models();
            let model = interactive::resolve_model(&models, model)?;
            display::model(&models, &model)
        }
        ModelCommand::Download {
            scope,
            model,
            version,
            out,
        } => {
            let console = session::connect(env)?;
            let (owner, project) = interactive::resolve_project(&console, scope)?;
            let models = console.project(&owner, &project).models();
            let model = interactive::resolve_model(&models, model)?;
            let version = interactive::resolve_version(&models, &model.name, version)?;
            let out = out.unwrap_or_else(|| format!("{}-v{}", model.name, version.version).into());
            progress::download(&models, &model.name, &version, &out)
        }
    }
}
