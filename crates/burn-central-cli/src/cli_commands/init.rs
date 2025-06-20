use crate::burn_dir::project::BurnCentralProject;
use crate::context::CliContext;
use crate::terminal::Terminal;
use crate::util::git;
use anyhow::Context;
use clap::Args;
use std::io::Write;

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Force reinitialization of the project
    #[arg(long, short = 'f')]
    pub force: bool,
}

pub fn handle_command(args: InitArgs, mut context: CliContext) -> anyhow::Result<()> {
    let client = super::login::get_client_and_login_if_needed(&mut context)
        .context("Failed to obtain the client")?;

    if !args.force && context.burn_dir().load_project().is_ok() {
        context
            .terminal()
            .print("Project already initialized. Use the `--force` flag to reinitialize.");
        return Ok(());
    }

    let ws_root = context
        .get_workspace_root()
        .context("Failed to get workspace root")?;

    cliclack::clear_screen()?;
    cliclack::intro(console::style("Project Initialization").black().on_blue())?;

    if !git::is_repo_initialized() {
        let repo = git::init_repo(&ws_root)?;
        cliclack::log::step(&format!(
            "No git repository found. Initialized new git repository at: {}",
            repo.path().display()
        ))?;
    }
    match git::is_repo_dirty() {
        Ok(false) => (),
        Ok(true) => {
            cliclack::log::info(
                "Repository is dirty. Please commit or stash your changes before proceeding.",
            )?;
            commit_sequence(&context)
                .map_err(|e| anyhow::anyhow!("Failed to make initial commit: {}", e))?;
        }
        Err(e) if e.to_string().contains("does not have any commits") => {
            cliclack::log::info(
                "Repository is dirty. Please commit or stash your changes before proceeding.",
            )?;
            commit_sequence(&context)
                .map_err(|e| anyhow::anyhow!("Failed to make initial commit: {}", e))?;
        }
        Err(_) => {
            return Err(anyhow::anyhow!(
                "Failed to check if the repository is dirty."
            ));
        }
    }
    let first_commit_hash = git::get_first_commit_hash();
    if let Err(e) = first_commit_hash {
        cliclack::outro_cancel(
            "No commits found in the repository. Please make an initial commit before proceeding.",
        )?;
        return Err(anyhow::anyhow!("Failed to get first commit hash: {}", e));
    }
    let first_commit_hash = first_commit_hash?;

    let user_name = client.get_current_user()?.username;

    // TODO: Fetch available namespaces from the server
    let available_namespaces = [
        (&user_name, format!("[user] {}", &user_name), ""),
        (
            &"my-organisation".to_string(),
            "[org] my-organization".to_string(),
            "",
        ),
        (&"tracel-ai".to_string(), "[org] tracel-ai".to_string(), ""),
        (
            &"burn-central-dev".to_string(),
            "[org] burn-central-dev".to_string(),
            "",
        ),
    ];
    let owner_name = cliclack::select("Select the owner of the project")
        .items(&available_namespaces)
        .initial_value(&available_namespaces[0].clone().0)
        .interact()?;

    let project_name = {
        let input = cliclack::input(&format!(
            "Enter the project name (default: {}) ",
            console::style(&context.metadata().user_crate_name).bold()
        ))
        .placeholder(&context.metadata().user_crate_name)
        .required(false)
        .validate(|input: &String| {
            if input.is_empty() {
                Ok(())
            } else if input
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            {
                Ok(())
            } else {
                Err("Project name must be alphanumeric or contain underscores only.".to_string())
            }
        })
        .interact::<String>()?;

        let project_name = if input.is_empty() {
            context.metadata().user_crate_name.clone()
        } else {
            input
        };
        project_name
    };

    // create the burn central project here
    let created_project =
        client.create_project(&owner_name, &project_name, Some(&first_commit_hash));
    if let Err(e) = created_project {
        cliclack::outro_cancel(&format!("Failed to create project: {}", e))?;
        return Err(anyhow::anyhow!("Failed to create project: {}", e));
    }

    context.burn_dir().save_project(&BurnCentralProject {
        name: project_name.clone(),
        owner: owner_name.clone(),
        git: first_commit_hash,
    })?;
    cliclack::log::success("Created project metadata")?;

    let frontend_url =
        &format!("https://central.burn.dev/{}/{}", owner_name, project_name).parse()?;
    cliclack::outro(&format!(
        "Project initialized successfully! You can check out your project at {}",
        Terminal::url(frontend_url)
    ))?;

    Ok(())
}

pub fn commit_sequence(context: &CliContext) -> anyhow::Result<()> {
    let do_commit = cliclack::confirm("Automatically commit all files?").interact()?;
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
            console::style("Manual commit").bold(),
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
                Ok(console::Key::Enter) => {
                    if !git::is_repo_dirty().is_ok() {
                        spinner.stop("Manual commit");
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    spinner.error("Manual commit");
                    cliclack::outro_cancel("Interrupted")?;
                    return Err(anyhow::anyhow!("Manual commit interrupted"));
                }
                _ => continue,
            }
        }
    }

    Ok(())
}
