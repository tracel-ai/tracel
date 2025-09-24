use crate::context::CliContext;

pub fn handle_command(context: CliContext) -> anyhow::Result<()> {
    context.terminal().command_title("Unlink");

    let confirm_value = context
        .terminal()
        .confirm("Are you sure you want to unlink the burn central project to this repository?")?;

    if confirm_value {
        context.burn_dir().delete()?;
        context.terminal().finalize("Project unlinked");
    } else {
        context.terminal().cancel_finalize("Cancelled");
        return Err(anyhow::anyhow!("Cancelled"));
    }

    Ok(())
}
