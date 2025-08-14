use crate::context::CliContext;
use crate::print_success;
use crate::registry::get_registered_functions;
use crate::util::git::{get_last_commit_hash, is_repo_dirty};
use burn_central_client::schemas::{BurnCentralCodeMetadata, PackagedCrateData};
use clap::Args;

#[derive(Args, Debug)]
pub struct PackageArgs {
    /// Name of the package to upload
    #[arg(long, action)]
    pub allow_dirty: bool,
}

pub(crate) fn handle_command(args: PackageArgs, context: CliContext) -> anyhow::Result<()> {

    if is_repo_dirty()? && !args.allow_dirty {
        anyhow::bail!("Repo is dirty. Please commit or stash your changes before packaging.");
    }
    let last_commit_hash = get_last_commit_hash()?;

    let client = context.create_client()?;
    let crates = package(
        &context.get_artifacts_dir_path(),
        context.package_name(),
    )?;

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
        crates,
        &last_commit_hash,
    )?;

    print_success!("New project version uploaded: {}", project_version);

    Ok(())
}

pub fn package(artifacts_dir: &std::path::Path, package_name: &str) -> anyhow::Result<Vec<PackagedCrateData>> {
    let crates = crate::util::cargo::package::package(artifacts_dir, package_name)?;
    Ok(crates)
}
