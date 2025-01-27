use crate::context::HeatCliCrateContext;
use crate::registry::get_registered_functions;
use crate::{print_err, print_success};
use clap::Parser;
use heat_sdk::schemas::HeatCodeMetadata;

#[derive(Parser, Debug)]
pub struct PackageArgs {
    /// The Heat project ID
    // todo: support project name and creating a project if it doesn't exist
    #[clap(
        short = 'p',
        long = "project",
        required = true,
        help = "<required> The Heat project path. Ex: test/Default-Project"
    )]
    project_path: String,
    /// The Heat API key
    #[clap(short = 'k', long = "key", help = "<required> The Heat API key")]
    key: Option<String>,
}

pub(crate) fn handle_command(
    args: PackageArgs,
    context: &mut HeatCliCrateContext,
) -> anyhow::Result<()> {
    let last_commit_hash = get_last_commit_hash()?;

    let heat_client = context.create_heat_client(
        // &args.key,
        // context.get_api_endpoint().as_str(),
        args.key.clone(),
        &args.project_path,
    )?;

    let crates = crate::util::cargo::package::package(
        &context.get_artifacts_dir_path(),
        context.package_name(),
    )?;

    let registered_functions = get_registered_functions();

    let heat_metadata = HeatCodeMetadata {
        functions: registered_functions,
    };

    let project_version = heat_client.upload_new_project_version(
        context.package_name(),
        heat_metadata,
        crates,
        &last_commit_hash,
    )?;

    print_success!("New project version uploaded: {}", project_version);

    Ok(())
}

fn get_last_commit_hash() -> anyhow::Result<String> {
    let repo = gix::discover(".")?;
    let last_commit = repo.head()?.peel_to_commit_in_place()?.id();
    if repo.is_dirty()? {
        print_err!("Latest git commit: {}", last_commit);
        anyhow::bail!("Repo is dirty. Please commit or stash your changes before packaging.");
    }

    Ok(last_commit.to_string())
}
