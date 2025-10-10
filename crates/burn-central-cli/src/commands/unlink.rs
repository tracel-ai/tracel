use crate::context::CliContext;

pub fn handle_command(context: CliContext) {
    context.terminal().command_title("Unlink");

    let confirm_value = context
        .terminal()
        .confirm("Are you sure you want to unlink the burn central project to this repository?")
        .unwrap();

    if confirm_value {
        match context.burn_dir().unlink_project() {
            Ok(_) => context.terminal().finalize("Project unlinked"),
            Err(e) => {
                context
                    .terminal()
                    .cancel_finalize(&format!("Failed to unlink project: {}", e));
            }
        }
    } else {
        context.terminal().cancel_finalize("Cancelled");
    }
}
