use crate::app_config::Environment;
use crate::commands::login::get_client_and_login_if_needed;
use crate::context::CliContext;

pub fn handle_command(mut context: CliContext) -> anyhow::Result<()> {
    context.terminal().command_title("User Information");

    let client = get_client_and_login_if_needed(&mut context);
    if let Err(e) = client {
        context.terminal().cancel_finalize(&format!(
            "Failed to connect to the server: {}. Please run 'cargo run -- login' to authenticate.",
            &e
        ));
        anyhow::bail!(e);
    }
    let client = client.unwrap();

    let user = match client.get_current_user() {
        Ok(user) => user,
        Err(e) => {
            context
                .terminal()
                .cancel_finalize(&format!("Failed to retrieve user information: {}", e));
            anyhow::bail!(e);
        }
    };

    context
        .terminal()
        .print(&format!("Username: {}", user.username));
    context.terminal().print(&format!("Email: {}", user.email));
    context
        .terminal()
        .print(&format!("Namespace: {}", user.namespace));

    let env_name = match context.environment() {
        Environment::Development => "Development (localhost)",
        Environment::Production => "Production (heat.tracel.ai)",
    };
    context
        .terminal()
        .print(&format!("Environment: {}", env_name));
    context
        .terminal()
        .print(&format!("API Endpoint: {}", context.get_api_endpoint()));

    context
        .terminal()
        .finalize("User information retrieved successfully.");

    Ok(())
}
