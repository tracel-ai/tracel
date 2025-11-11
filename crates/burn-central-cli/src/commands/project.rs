use crate::commands::login::get_client_and_login_if_needed;
use crate::context::CliContext;

pub fn handle_command(mut context: CliContext) {
    context.terminal().command_title("Project Information");

    // Load the local project metadata
    if context.load_project().is_err() {
        context.terminal().cancel_finalize(
            "Project is not configured. Please run 'cargo run -- init' to link a project.",
        );
        return;
    }

    let client = match get_client_and_login_if_needed(&mut context) {
        Ok(client) => client,
        Err(_) => {
            context.terminal().cancel_finalize(
                "Failed to connect to the server. Please run 'cargo run -- login' to authenticate.",
            );
            return;
        }
    };

    // Get the project path (owner and name) from local metadata
    let project_path = match context.get_project_path() {
        Ok(path) => path,
        Err(_) => {
            context.terminal().cancel_finalize(
                "Project is not configured. Please run 'cargo run -- init' to link a project.",
            );
            return;
        }
    };

    // Fetch project information from the server
    match client.get_project(project_path.owner_name(), project_path.project_name()) {
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
                project_path.owner_name(),
                project_path.project_name()
            ));
        }
        Err(e) => {
            context
                .terminal()
                .cancel_finalize(&format!("Failed to retrieve project information: {}", e));
        }
    };
}
