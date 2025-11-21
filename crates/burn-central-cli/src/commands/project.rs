use crate::commands::login::get_client_and_login_if_needed;
use crate::context::CliContext;
use crate::helpers::require_linked_project;

pub fn handle_command(mut context: CliContext) -> anyhow::Result<()> {
    context.terminal().command_title("Project Information");

    let project = require_linked_project(&context)?;

    let client = match get_client_and_login_if_needed(&mut context) {
        Ok(client) => client,
        Err(_) => {
            context.terminal().cancel_finalize(
                "Failed to connect to the server. Please run 'burn login' to authenticate.",
            );
            return Ok(());
        }
    };

    let bc_project = project.get_project();
    // Fetch project information from the server
    match client.get_project(&bc_project.owner, &bc_project.name) {
        Ok(project) => {
            context
                .terminal()
                .print(&format!("Project: {}", project.project_name));
            context
                .terminal()
                .print(&format!("Namespace: {}", project.namespace_name));
            context
                .terminal()
                .print(&format!("Description: {}", project.description));
            context
                .terminal()
                .print(&format!("Created By: {}", project.created_by));
            context
                .terminal()
                .finalize("Project information retrieved successfully.");
        }
        Err(e) if e.is_not_found() => {
            context.terminal().cancel_finalize(&format!(
                "Project {}/{} not found on the server.",
                &bc_project.owner, &bc_project.name
            ));
        }
        Err(e) => {
            context
                .terminal()
                .cancel_finalize(&format!("Failed to retrieve project information: {}", e));
        }
    };

    Ok(())
}
