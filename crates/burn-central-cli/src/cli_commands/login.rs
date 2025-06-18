use crate::app_config::Credentials;
use crate::context::CliContext;
use anyhow::Context;
use clap::Args;
use std::io;
use std::io::Write;

fn format_console_url(url: &url::Url) -> String {
    format!("\x1b[1;34m{}\x1b[0m", url)
}

#[derive(Args, Debug)]
pub struct LoginArgs {
    #[arg(long)]
    pub api_key: Option<String>,
}

fn prompt_api_key() -> Option<String> {
    print!("? ");
    io::stdout().flush().unwrap();

    let mut input = String::new();
    match io::stdin().read_line(&mut input) {
        Ok(_) => {
            let trimmed = input.trim();
            if trimmed.is_empty() {
                println!("Input cancelled.");
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Err(err) => {
            eprintln!("Error reading input: {err}");
            None
        }
    }
}

pub fn handle_command(args: LoginArgs, mut context: CliContext) -> anyhow::Result<()> {
    if let Some(api_key) = args.api_key {
        context.set_credentials(Credentials { api_key });
    } else {
        print!(
            "Enter your API key found on {} below:",
            format_console_url(&context.get_api_endpoint().join("me")?)
        );
        let api_key = prompt_api_key();
        if let Some(key) = api_key {
            context.set_credentials(Credentials { api_key: key });
        } else {
            println!("Login cancelled.");
            return Ok(());
        }
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
