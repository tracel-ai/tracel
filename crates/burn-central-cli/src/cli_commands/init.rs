use crate::burn_dir::project::BurnCentralProject;
use crate::context::CliContext;
use crate::util::git;
use anyhow::Context;
use clap::Args;

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

    if !git::is_repo_initialized() {
        context
            .terminal()
            .print("No git repository found. Initializing a new git repository.");
        let repo = git::init_repo(&ws_root)?;
        context.terminal().print(&format!(
            "Initialized new git repository at: {}",
            repo.path().display()
        ));
    }
    let first_commit_hash = match git::get_first_commit_hash() {
        Ok(first_commit_hash) => {
            context
                .terminal()
                .print(&format!("First commit hash: {}", first_commit_hash));
            first_commit_hash
        }
        Err(e) => {
            if !e.to_string().contains("does not have any commits") {
                return Err(e);
            }
            context.terminal().print("No commits found in the repository. Please make an initial commit before proceeding.");
            match commit_sequence(&context) {
                Ok(_) => {
                    context
                        .terminal()
                        .print("Initial commit made successfully.");
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to make initial commit: {}", e));
                }
            }
            git::get_first_commit_hash()
                .context("Failed to get first commit hash after initial commit")?
        }
    };

    let input_project_name = loop {
        let input = context
            .terminal()
            .read_line(&format!(
                "Enter the project name (default: {}) ",
                console::style(&context.metadata().user_crate_name).bold()
            ))
            .map(|s| s.trim().to_string())
            .context("Failed to read project name")?;

        let project_name = if input.is_empty() {
            context.metadata().user_crate_name.clone()
        } else {
            input
        };

        if project_name.is_empty() {
            context
                .terminal()
                .print("Project name cannot be empty. Please try again.");
        } else {
            break project_name;
        }
    };

    context.terminal().print("Creating project metadata...");
    context.burn_dir().save_project(&BurnCentralProject {
        name: input_project_name,
        owner: client.get_current_user()?.username,
        git: first_commit_hash,
    })?;

    Ok(())
}

pub fn commit_sequence(context: &CliContext) -> anyhow::Result<()> {
    let do_commit = loop {
        match context
            .terminal()
            .read_confirmation("Do you want to automatically commit all files? (y/n) ")
        {
            Ok(value) => break value,
            Err(e) => {
                context.terminal().print(&format!(
                    "Failed to read confirmation: {}. Please try again.",
                    e
                ));
            }
        }
    };
    if do_commit {
        let commit_message = "Initial commit by Burn Central CLI";
        let status = std::process::Command::new("git")
            .args(["add", "."])
            .status()
            .context("Failed to run `git add .`")?;
        if !status.success() {
            return Err(anyhow::anyhow!("Failed to add files to git"));
        }
        let status = std::process::Command::new("git")
            .args(["commit", "-m", commit_message])
            .status()
            .context("Failed to run `git commit -m`")?;
        if !status.success() {
            return Err(anyhow::anyhow!("Failed to commit files to git"));
        }
        context.terminal().print("Committed all files to git.");
    } else {
        loop {
            context
                .terminal()
                .print("Please make an initial commit before proceeding. Press any key to try again.");
            context.terminal().wait_for_keypress()?;
            if git::get_first_commit_hash().is_ok() {
                break;
            }
        }
    }

    Ok(())
}
