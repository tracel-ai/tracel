use crate::app_config::Credentials;
use crate::context::{CliContext, ClientCreationError};
use crate::terminal::Terminal;
use anyhow::Context;
use burn_central_client::BurnCentral;
use clap::Args;

#[derive(Args, Debug)]
pub struct LoginArgs {
    #[arg(long)]
    pub api_key: Option<String>,
}

pub fn prompt_login(context: &mut CliContext) -> anyhow::Result<()> {
    let prompt = format!(
        "Enter your API key found on {} below.",
        Terminal::url(&context.get_frontend_endpoint().join("/settings/api-keys")?),
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

pub fn get_client_and_login_if_needed(context: &mut CliContext) -> anyhow::Result<BurnCentral> {
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
                        prompt_login(context)?;
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

pub fn handle_command(args: LoginArgs, mut context: CliContext) -> anyhow::Result<()> {
    if let Some(api_key) = args.api_key {
        context.set_credentials(Credentials { api_key });
    } else {
        prompt_login(&mut context).context("Failed to prompt for API key")?;
    }

    let client = context
        .create_client()
        .context("Failed to authenticate with the server")?;
    let user = client.me().context("Failed to retrieve current user")?;
    println!("Successfully logged in! Welcome {}.", user.username);
    Ok(())
}
