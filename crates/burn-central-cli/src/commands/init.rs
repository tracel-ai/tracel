use crate::context::CliContext;
use crate::helpers::{can_initialize_project, require_rust_project};
use crate::tools::terminal::Terminal;
use anyhow::Context;
use burn_central_client::Client;
use burn_central_client::response::ProjectResponse;
use burn_central_workspace::tools::git;
use burn_central_workspace::{BurnCentralProject, ProjectContext, ProjectPath};
use clap::Args;

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Force reinitialization of the project
    #[arg(long, short = 'f')]
    pub force: bool,
}

pub fn handle_command(args: InitArgs, mut context: CliContext) -> anyhow::Result<()> {
    if !can_initialize_project(&context, args.force)? {
        return Ok(());
    }

    let client = super::login::get_client_and_login_if_needed(&mut context)?;
    prompt_init(&context, &client).context("Failed to initialize the project")
}

pub fn prompt_init(context: &CliContext, client: &Client) -> anyhow::Result<()> {
    let user = client.get_current_user()?;
    let crate_info = require_rust_project(context)?;

    context.terminal().command_title("Project Initialization");

    let terminal = context.terminal();

    ensure_git_repo_initialized(&crate_info.get_ws_root())?;
    ensure_git_repo_clean(terminal)?;

    let first_commit_hash = git::get_first_commit_hash();
    if let Err(e) = first_commit_hash {
        terminal.cancel_finalize(
            "No commits found in the repository. Please make an initial commit before proceeding.",
        );
        return Err(anyhow::anyhow!("Failed to get first commit hash: {}", e));
    }
    let _first_commit_hash = first_commit_hash?;

    let project_owner = prompt_owner_name(&user.username, client)?;
    let project_name = prompt_project_name(&crate_info.user_crate_name)?;

    let owner_name = match &project_owner {
        ProjectKind::User => user.namespace.as_str(),
        ProjectKind::Organization(org_name) => org_name.as_str(),
    };
    let project_path = match client.get_project(owner_name, &project_name) {
        Ok(project) => handle_existing_project(&project)?,
        Err(e) if e.is_not_found() => {
            create_new_project(client, project_owner.clone(), &project_name)?
        }
        Err(e) => {
            terminal.cancel_finalize(&format!("Failed to check for existing project: {e}"));
            return Err(anyhow::anyhow!(e));
        }
    };

    let project_name = project_path.project_name();
    let owner_name = project_path.owner_name();

    ProjectContext::init(
        BurnCentralProject {
            name: project_name.to_string(),
            owner: owner_name.to_string(),
        },
        &crate_info.get_manifest_path(),
        context.get_burn_dir_name(),
    )
    .map_err(|e| {
        terminal.cancel_finalize(&format!("Failed to initialize project metadata: {}", e));
        e
    })?;
    terminal.print("Created project metadata");

    let url_path = match &project_owner {
        ProjectKind::User => format!("/users/{}/projects/{}", user.username, project_name),
        ProjectKind::Organization(org_name) => {
            format!("/orgs/{}/projects/{}", org_name, project_name)
        }
    };
    let frontend_url = context
        .get_frontend_endpoint()
        .join(&url_path)
        .expect("Should be able to construct frontend URL");

    terminal.finalize(&format!(
        "Project initialized successfully! You can check out your project at {}",
        context.terminal().format_url(&frontend_url)
    ));

    Ok(())
}

fn prompt_owner_name(user_name: &str, client: &Client) -> anyhow::Result<ProjectKind> {
    let organizations = client.get_user_organizations()?;
    let mut namespaces = vec![(ProjectKind::User, format!("[user] {user_name}"), "")];
    namespaces.extend(organizations.organizations.into_iter().map(|org| {
        (
            ProjectKind::Organization(org.namespace.clone()),
            format!("[org] {}", org.name),
            "",
        )
    }));
    cliclack::select("Select the owner of the project")
        .items(&namespaces)
        .initial_value(ProjectKind::User)
        .interact()
        .map_err(anyhow::Error::from)
}

pub fn prompt_project_name(crate_name: &str) -> anyhow::Result<String> {
    let input = cliclack::input(format!(
        "Enter the project name (default: {}) ",
        console::style(crate_name).bold()
    ))
    .placeholder(crate_name)
    .required(false)
    .validate(|input: &String| {
        if input.is_empty()
            || input
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            Ok(())
        } else {
            Err("Project name must be alphanumeric or contain underscores only.".to_string())
        }
    })
    .interact::<String>()?;

    let input = if input.is_empty() {
        crate_name.to_string()
    } else {
        input
    };

    Ok(input)
}

fn handle_existing_project(project: &ProjectResponse) -> anyhow::Result<ProjectPath> {
    let confirmed = cliclack::confirm(format!(
        "Project \"{}\" already exists under owner \"{}\". Do you want to link it?",
        project.project_name, project.namespace_name
    ))
    .interact()?;

    if confirmed {
        Ok(ProjectPath::new(
            project.namespace_name.clone(),
            project.project_name.clone(),
        ))
    } else {
        cliclack::outro_cancel("Project initialization cancelled")?;
        Err(anyhow::anyhow!("Project initialization cancelled by user"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProjectKind {
    User,
    Organization(String),
}

fn create_new_project(
    client: &Client,
    project_kind: ProjectKind,
    name: &str,
) -> anyhow::Result<ProjectPath> {
    let description = cliclack::input("Enter the project description (default empty)")
        .required(false)
        .interact::<String>()?;
    let desc = if description.is_empty() {
        None
    } else {
        Some(description)
    };

    let created_project_path = match project_kind {
        ProjectKind::User => client.create_user_project(name, desc.as_deref()),
        ProjectKind::Organization(org_name) => {
            client.create_organization_project(&org_name, name, desc.as_deref())
        }
    };

    match created_project_path {
        Ok(project) => Ok(ProjectPath::new(
            project.namespace_name,
            project.project_name,
        )),
        Err(e) => {
            cliclack::outro_cancel(format!("Failed to create project: {e}"))?;
            Err(anyhow::anyhow!("Failed to create project: {}", e))
        }
    }
}

pub fn ensure_git_repo_initialized(ws_root: &std::path::Path) -> anyhow::Result<()> {
    if !git::is_repo_initialized() {
        let repo = git::init_repo(ws_root)?;
        cliclack::log::step(format!(
            "No git repository found. Initialized new git repository at: {}",
            repo.path().display()
        ))?;
    }
    Ok(())
}

pub fn ensure_git_repo_clean(terminal: &Terminal) -> anyhow::Result<()> {
    match git::is_repo_dirty() {
        Ok(false) => Ok(()),
        Ok(true) => {
            terminal.print(
                "Repository is dirty. Burn central needs a valid commit hash to associated your code with your repository.",
            );
            commit_sequence().map_err(|e| anyhow::anyhow!("Failed to make initial commit: {}", e))
        }
        Err(e) if e.to_string().contains("does not have any commits") => {
            terminal.print(
                "Repository is dirty. Please commit or stash your changes before proceeding.",
            );
            commit_sequence().map_err(|e| anyhow::anyhow!("Failed to make initial commit: {}", e))
        }
        Err(_) => Err(anyhow::anyhow!(
            "Failed to check if the repository is dirty."
        )),
    }
}

pub fn commit_sequence() -> anyhow::Result<()> {
    let do_commit =
        cliclack::confirm("Do you want to automatically commit all files?").interact()?;
    if do_commit {
        let commit_message = "Automatic commit by Burn Central CLI";
        let status = std::process::Command::new("git")
            .args(["add", "."])
            .stderr(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .status()
            .context("Failed to run `git add .`")?;
        if !status.success() {
            return Err(anyhow::anyhow!("Failed to add files to git"));
        }
        let status = std::process::Command::new("git")
            .args(["commit", "-m", commit_message])
            .stderr(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .status()
            .context("Failed to run `git commit -m`")?;
        if !status.success() {
            return Err(anyhow::anyhow!("Failed to commit files to git"));
        }
        cliclack::log::success("Committed all files to git.")?;
    } else {
        let spinner = cliclack::spinner();
        let message = format!(
            "{}\n{}\n\n{}",
            console::style("Waiting for manual commit").bold(),
            console::style("Press Esc, Enter, or Ctrl-C").dim(),
            console::style(
                "Please make a commit before proceeding. Press Enter to continue or Esc to cancel."
            )
            .magenta()
            .italic()
        );
        spinner.start(message);
        let term = console::Term::stderr();
        loop {
            match term.read_key() {
                Ok(console::Key::Escape) => {
                    spinner.cancel("Manual commit");
                    cliclack::outro_cancel("Cancelled")?;
                    return Err(anyhow::anyhow!("Manual commit cancelled"));
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    spinner.error("Manual commit");
                    cliclack::outro_cancel("Interrupted")?;
                    return Err(anyhow::anyhow!("Manual commit interrupted"));
                }
                _ => {
                    if let Ok(false) | Err(_) = git::is_repo_dirty() {
                        spinner.stop("Manual commit");
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}
