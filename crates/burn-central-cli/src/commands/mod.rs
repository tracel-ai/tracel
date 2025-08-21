use crate::commands::init::prompt_init;
use crate::commands::training::TrainingArgs;
use crate::print_info;
use crate::{commands::login::get_client_and_login_if_needed, context::CliContext};
pub mod init;
pub mod login;
pub mod package;
pub mod training;

pub fn default_command(mut context: CliContext) -> anyhow::Result<()> {
    let project_loaded = context.load_project().is_ok();

    let client = get_client_and_login_if_needed(&mut context)?;

    if !project_loaded {
        print_info!("No project loaded. Running initialization sequence.");
        prompt_init(&context, &client)?;
    } else {
        training::handle_command(
            TrainingArgs {
                function: None,
                config: None,
                overrides: vec![],
                project_version: None,
                runner: None,
                backend: None,
            },
            context,
        )?;
    }

    Ok(())
}
