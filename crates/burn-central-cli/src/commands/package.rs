use std::path::PathBuf;

use crate::commands::init::ensure_git_repo_clean;
use crate::context::CliContext;
use crate::print_success;
use crate::tools::cargo::package::package;
use crate::tools::git::is_repo_dirty;
use anyhow::Context;
use burn_central_api::Client;
use burn_central_api::schemas::{BurnCentralCodeMetadata, CrateVersionMetadata, PackagedCrateData};
use clap::Args;

#[derive(Args, Debug)]
pub struct PackageArgs {
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
        ensure_git_repo_clean(context.terminal())?;
    }

    let client = context.create_client()?;
    let package = package(&context.get_artifacts_dir_path(), context.package_name())?;

    let registered_functions = context.function_registry.get_registered_functions();

    let code_metadata = BurnCentralCodeMetadata {
        functions: registered_functions,
    };

    let project_path = context.get_project_path()?;
    let digest = upload_new_project_version(
        client,
        project_path.owner_name(),
        project_path.project_name(),
        context.package_name(),
        code_metadata,
        package.crate_metadata,
        &package.digest,
    )?;

    Ok(digest)
}

/// Upload a new version of a project to Burn Central.
pub fn upload_new_project_version(
    ref client: Client,
    namespace: &str,
    project_name: &str,
    target_package_name: &str,
    code_metadata: BurnCentralCodeMetadata,
    crates_data: Vec<PackagedCrateData>,
    last_commit: &str,
) -> anyhow::Result<String> {
    let (data, metadata): (Vec<(String, PathBuf)>, Vec<CrateVersionMetadata>) = crates_data
        .into_iter()
        .map(|krate| {
            (
                (krate.name, krate.path),
                CrateVersionMetadata {
                    checksum: krate.checksum,
                    metadata: krate.metadata,
                    size: krate.size,
                },
            )
        })
        .unzip();

    let response = client
        .publish_project_version_urls(
            namespace,
            project_name,
            target_package_name,
            code_metadata,
            metadata,
            last_commit,
        )
        .with_context(|| {
            format!("Failed to get upload URLs for project {namespace}/{project_name}")
        })?;

    if let Some(urls) = response.urls {
        for (crate_name, file_path) in data.into_iter() {
            let url = urls
                .get(&crate_name)
                .ok_or_else(|| anyhow::anyhow!("No upload URL found for crate: {crate_name}"))?;

            let data = std::fs::read(&file_path).map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!("Failed to read crate file {}: {}", file_path.display(), e),
                )
            })?;

            client
                .upload_bytes_to_url(url, data)
                .with_context(|| format!("Failed to upload crate {crate_name} to URL {url}"))?;
        }

        client
            .complete_project_version_upload(namespace, project_name, &response.id)
            .with_context(|| {
                format!("Failed to complete upload for project {namespace}/{project_name}")
            })?;
    }

    Ok(response.digest)
}
