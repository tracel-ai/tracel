use crate::commands::login::get_client_and_login_if_needed;
use crate::context::CliContext;

pub fn handle_command(mut context: CliContext) {
    context.terminal().command_title("Project Information");

    // Load the local project metadata
    if let Err(_) = context.load_project() {
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
    let project = match client.find_project(project_path.owner_name(), project_path.project_name())
    {
        Ok(project) => project,
        Err(e) => {
            context
                .terminal()
                .cancel_finalize(&format!("Failed to retrieve project information: {}", e));
            return;
        }
    };

    match project {
        Some(project_info) => {
            context
                .terminal()
                .print(&format!("Project: {}", project_info.project_name));
            context
                .terminal()
                .print(&format!("Namespace: {}", project_info.namespace_name));
            context
                .terminal()
                .print(&format!("Description: {}", project_info.description));
            context
                .terminal()
                .print(&format!("Created By: {}", project_info.created_by));
            context
                .terminal()
                .finalize("Project information retrieved successfully.");
        }
        None => {
            context.terminal().cancel_finalize(&format!(
                "Project {}/{} not found on the server.",
                project_path.owner_name(),
                project_path.project_name()
            ));
        }
    }
}
