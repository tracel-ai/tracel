use anyhow::Context;
use clap::Parser;
use crate::context::HeatCliContext;
use inquire::Password;
use keyring::Entry;

#[derive(Parser, Debug)]
pub struct LoginArgs {}

pub(crate) fn handle_command(
    _args: LoginArgs,
    _context: &mut HeatCliContext,
) -> anyhow::Result<()> {
    let api_key = Password::new("Enter your API key:")
        .without_confirmation()
        .prompt()
        .context("Failed to read the API key")?;

    let entry = Entry::new("heat-sdk-cli", "api_key").context("Failed to create a keyring entry")?;
    entry.set_password(&api_key).context("Failed to store the API key")?;
    
    println!("API key stored successfully");

    Ok(())
}
