use crate::commands::init::prompt_init;
use crate::commands::training::TrainingArgs;
use crate::helpers::{is_burn_central_project_linked, require_rust_project};
use crate::{commands::login::get_client_and_login_if_needed, context::CliContext};

pub mod init;
pub mod login;
pub mod me;
pub mod package;
pub mod project;
pub mod training;
pub mod unlink;

pub fn default_command(mut context: CliContext) -> anyhow::Result<()> {
    let client = get_client_and_login_if_needed(&mut context)?;

    // Check if we have a linked Burn Central project
    if !is_burn_central_project_linked(&context) {
        // Make sure we're at least in a Rust project before initializing
        let _crate_info = require_rust_project(&context)?;
        context
            .terminal()
            .print("No Burn Central project linked, prompting for initialization.");
        prompt_init(&context, &client)?;
        return Ok(());
    }

    training::handle_command(TrainingArgs::default(), context)?;

    Ok(())
}
