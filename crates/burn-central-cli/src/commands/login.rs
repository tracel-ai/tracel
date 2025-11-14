use crate::app_config::Credentials;
use crate::context::{CliContext, ClientCreationError};
use anyhow::Context;
use burn_central_client::Client;
use clap::Args;

#[derive(Args, Debug)]
pub struct LoginArgs {
    #[arg(long)]
    pub api_key: Option<String>,
}

pub fn get_client_and_login_if_needed(context: &mut CliContext) -> anyhow::Result<Client> {
    const MAX_RETRIES: u32 = 3;
    let mut attempts = 0;

    loop {
        match context.create_client() {
            Ok(client) => {
                if attempts > 0 {
                    context.terminal().print("Successfully logged in!");
                }
                return Ok(client);
            }
            Err(err) => {
                attempts += 1;
                match err {
                    ClientCreationError::InvalidCredentials
                    | ClientCreationError::NoCredentials => {
                        if attempts > MAX_RETRIES {
                            return Err(anyhow::anyhow!("Maximum login attempts exceeded"));
                        }
                        context
                            .terminal()
                            .print("Failed to login. Please try again. Press Ctrl+C to exit.");

                        let api_key = prompt_login(context)?;

                        context.set_credentials(Credentials { api_key });

                        context
                            .create_client()
                            .context("Failed to authenticate with the server")?;
                    }
                    ClientCreationError::ServerConnectionError(msg) => {
                        if attempts > MAX_RETRIES {
                            return Err(anyhow::anyhow!(
                                "Server connection failed after maximum retries: {}",
                                msg
                            ));
                        }
                        context.terminal().print(&format!(
                            "Failed to connect to the server: {msg}. Retrying..."
                        ));
                    }
                }
            }
        }
    }
}

pub fn prompt_login(context: &mut CliContext) -> anyhow::Result<String> {
    context.terminal().input_password(&format!(
        "Enter your API key found on {} below.",
        context
            .terminal()
            .format_url(&context.get_frontend_endpoint().join("/settings/api-keys")?),
    ))
}

pub fn handle_command(args: LoginArgs, mut context: CliContext) -> anyhow::Result<()> {
    let api_key = match args.api_key {
        Some(api_key) => api_key,
        None => {
            context
                .terminal()
                .command_title("Credential initialization");
            prompt_login(&mut context)?
        }
    };

    context.set_credentials(Credentials { api_key });

    let mut client = context.create_client();

    while client.is_err() {
        context
            .terminal()
            .print_err("Invalid credentials. Please try again.");
        let api_key = prompt_login(&mut context)?;
        context.set_credentials(Credentials { api_key });
        client = context.create_client();
    }

    let user = client.unwrap().get_current_user();
    if let Ok(user) = user {
        context.terminal().finalize(&format!(
            "Successfully logged in! Welcome {}.",
            user.username
        ));
    } else {
        context
            .terminal()
            .cancel_finalize("Login failed, invalid credentials!");
    }

    Ok(())
}
