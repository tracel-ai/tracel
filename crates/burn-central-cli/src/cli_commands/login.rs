use crate::app_config::Credentials;
use crate::context::{CliContext, ClientCreationError};
use anyhow::Context;
use clap::Args;
use std::io;
use std::io::Write;
use burn_central_client::client::BurnCentralClient;

fn format_console_url(url: &url::Url) -> String {
    format!("\x1b[1;34m{}\x1b[0m", url)
}

#[derive(Args, Debug)]
pub struct LoginArgs {
    #[arg(long)]
    pub api_key: Option<String>,
}

pub fn prompt_login(context: &mut CliContext) -> anyhow::Result<()> {
    let prompt = format!(
        "Enter your API key found on {} below:\n",
        format_console_url(&context.get_api_endpoint().join("me")?)
    );
    let api_key = context.terminal().read_password(Some(&prompt))?;
    if !api_key.trim().is_empty() {
        context.set_credentials(Credentials { api_key });
    } else {
        println!("Login cancelled.");
        return Ok(());
    }

    Ok(())
}

pub fn get_client_and_login_if_needed(context: &mut CliContext) -> anyhow::Result<BurnCentralClient> {
    let client_res = context.create_client();
    if let Err(err) = client_res {
        match err {
            ClientCreationError::NoCredentials => {
                context
                    .terminal()
                    .print("No credentials found.");
                prompt_login(
                    context,
                )?;
                let client = context
                    .create_client()
                    .context("Failed to authenticate with the server")?;
                Ok(client)
            }
            ClientCreationError::ServerConnectionError(ref msg) => {
                context
                    .terminal()
                    .print(&format!(
                        "Failed to connect to the server: {}.",
                        msg
                    ));
                Err(err.into())
            }
        }
    }
    else {
        Ok(client_res?)
    }
}

pub fn handle_command(args: LoginArgs, mut context: CliContext) -> anyhow::Result<()> {
    if let Some(api_key) = args.api_key {
        context.set_credentials(Credentials { api_key });
    } else {
        prompt_login(&mut context)
            .context("Failed to prompt for API key")?;
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
