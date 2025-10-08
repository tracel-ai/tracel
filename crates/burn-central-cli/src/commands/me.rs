use crate::commands::login::get_client_and_login_if_needed;
use crate::context::CliContext;

pub fn handle_command(mut context: CliContext) {
    context.terminal().command_title("User Information");

    let client = get_client_and_login_if_needed(&mut context);
    if let Err(e) = client {
        context.terminal().cancel_finalize(&format!(
            "Failed to connect to the server: {}. Please run 'cargo run -- login' to authenticate.",
            e
        ));
        return;
    }
    let client = client.unwrap();

    let user = match client.me() {
        Ok(user) => user,
        Err(e) => {
            context
                .terminal()
                .cancel_finalize(&format!("Failed to retrieve user information: {}", e));
            return;
        }
    };

    context
        .terminal()
        .print(&format!("Username: {}", user.username));
    context.terminal().print(&format!("Email: {}", user.email));
    context
        .terminal()
        .print(&format!("Namespace: {}", user.namespace));

    context
        .terminal()
        .finalize("User information retrieved successfully.");
}
