use burn_central_workspace::ProjectContext;

use crate::{context::CliContext, helpers::require_linked_project};

pub fn handle_command(context: CliContext) -> anyhow::Result<()> {
    let _project = require_linked_project(&context)?;

    context.terminal().command_title("Unlink");

    let confirm_value = context
        .terminal()
        .confirm("Are you sure you want to unlink the burn central project from this repository?")
        .unwrap();

    if confirm_value {
        match ProjectContext::unlink(context.get_burn_dir_name()) {
            Ok(_) => context.terminal().finalize("Project unlinked successfully"),
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
