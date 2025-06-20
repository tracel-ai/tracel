use crate::app_config::Credentials;
use crate::context::{CliContext, ClientCreationError};
use crate::terminal::Terminal;
use anyhow::Context;
use burn_central_client::client::BurnCentralClient;
use clap::Args;

#[derive(Args, Debug)]
pub struct LoginArgs {
    #[arg(long)]
    pub api_key: Option<String>,
}

pub fn prompt_login(context: &mut CliContext) -> anyhow::Result<()> {
    let prompt = format!(
        "Enter your API key found on {} below.",
        Terminal::url(&context.get_api_endpoint().join("me")?),
    );
    context.terminal().print(&prompt);
    let api_key = context.terminal_mut().read_password("Key")?;
    if !api_key.trim().is_empty() {
        context.set_credentials(Credentials { api_key });
    } else {
        println!("Login cancelled.");
        return Ok(());
    }

    Ok(())
}

#[allow(dead_code)]
pub fn get_client_and_login_if_needed(
    context: &mut CliContext,
) -> anyhow::Result<BurnCentralClient> {
    let client_res = context.create_client();
    while let Err(err) = &client_res {
        match err {
            ClientCreationError::InvalidCredentials | ClientCreationError::NoCredentials => {
                prompt_login(context)?;
                let client = context.create_client();
                match client {
                    Ok(client) => {
                        context.terminal().print("Successfully logged in!");
                        return Ok(client);
                    }
                    Err(e) => {
                        context.terminal().print(&format!(
                            "Failed to create client: {}. Please try again. Press Ctrl+C to exit.",
                            e
                        ));
                        continue;
                    }
                }
            }
            ClientCreationError::ServerConnectionError(msg) => {
                context
                    .terminal()
                    .print(&format!("Failed to connect to the server: {}.", msg));
                continue;
            }
        }
    }
    Ok(client_res?)
}

pub fn handle_command(args: LoginArgs, mut context: CliContext) -> anyhow::Result<()> {
    if let Some(api_key) = args.api_key {
        context.set_credentials(Credentials { api_key });
    } else {
        prompt_login(&mut context).context("Failed to prompt for API key")?;
    }

    let client = context
        .create_client()
        .context("Failed to authenticate with the server")?;
    let user = client
        .get_current_user()
        .context("Failed to retrieve current user")?;
    println!("Successfully logged in! Welcome {}.", user.username);
    Ok(())
}
