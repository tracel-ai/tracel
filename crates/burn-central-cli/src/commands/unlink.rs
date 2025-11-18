use crate::{context::CliContext, entity::projects::ProjectContext};

pub fn handle_command(context: CliContext) -> anyhow::Result<()> {
    let project = ProjectContext::discover(context.environment())?;

    if project.get_project().is_none() {
        context
            .terminal()
            .cancel_finalize("No burn central project is linked to this package.");
        return Ok(());
    }

    context.terminal().command_title("Unlink");

    let confirm_value = context
        .terminal()
        .confirm("Are you sure you want to unlink the burn central project to this repository?")
        .unwrap();

    if confirm_value {
        match project.burn_dir().unlink_project() {
            Ok(_) => context.terminal().finalize("Project unlinked"),
            Err(e) => {
                context
                    .terminal()
                    .cancel_finalize(&format!("Failed to unlink project: {}", e));
                anyhow::bail!(e);
            }
        }
    } else {
        context.terminal().cancel_finalize("Cancelled");
    }

    Ok(())
}
