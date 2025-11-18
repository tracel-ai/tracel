use crate::commands::init::prompt_init;
use crate::commands::training::TrainingArgs;
use crate::entity::projects::ProjectContext;
use crate::print_info;
use crate::{commands::login::get_client_and_login_if_needed, context::CliContext};
pub mod init;
pub mod login;
pub mod me;
pub mod package;
pub mod project;
pub mod training;
pub mod unlink;

pub fn default_command(mut context: CliContext) -> anyhow::Result<()> {
    let mut project = ProjectContext::discover(context.environment())?;
    let project_loaded = project.get_project().is_some();

    let client = get_client_and_login_if_needed(&mut context)?;

    if !project_loaded {
        print_info!("No project loaded. Running initialization sequence.");
        prompt_init(&context, &client, &mut project)?;
    } else {
        training::handle_command(TrainingArgs::default(), context)?;
    }

    Ok(())
}
