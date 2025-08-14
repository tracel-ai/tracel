use crate::context::CliContext;
use crate::print_success;
use crate::registry::get_registered_functions;
use crate::util::cargo::package::package;
use crate::util::git::is_repo_dirty;
use burn_central_client::schemas::BurnCentralCodeMetadata;
use clap::Args;

#[derive(Args, Debug)]
pub struct PackageArgs {
    /// Name of the package to upload
    #[arg(long, action)]
    pub allow_dirty: bool,
}

pub(crate) fn handle_command(args: PackageArgs, context: CliContext) -> anyhow::Result<()> {
    let version = package_sequence(&context, args.allow_dirty)?;
    print_success!("New project version uploaded: {version}");

    Ok(())
}

pub fn package_sequence(context: &CliContext, allow_dirty: bool) -> anyhow::Result<String> {
    if is_repo_dirty()? && !allow_dirty {
        anyhow::bail!("Repo is dirty. Please commit or stash your changes before packaging.");
    }

    let client = context.create_client()?;
    let package = package(&context.get_artifacts_dir_path(), context.package_name())?;

    let flags = crate::registry::get_flags();
    let registered_functions = get_registered_functions(&flags);

    let code_metadata = BurnCentralCodeMetadata {
        functions: registered_functions,
    };

    let project_path = context.get_project_path()?;
    let project_version = client.upload_new_project_version(
        project_path.owner_name(),
        project_path.project_name(),
        context.package_name(),
        code_metadata,
        package.crate_metadata,
        &package.digest,
    )?;

    Ok(project_version)
}
