use crate::context::HeatCliCrateContext;
use crate::registry::Flag;
use crate::util::git::{DefaultGitRepo, GitRepo};
use crate::print_success;
use clap::Parser;
use heat_sdk::schemas::{HeatCodeMetadata, RegisteredHeatFunction};
use quote::ToTokens;

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
    #[clap(
        short = 'k',
        long = "key",
        help = "<required> The Heat API key"
    )]
    key: Option<String>,
}

pub(crate) fn handle_command(args: PackageArgs, context: &mut HeatCliCrateContext) -> anyhow::Result<()> {
    let last_commit_hash = DefaultGitRepo::new()?
        .if_not_dirty()?
        .get_last_commit_hash()?;

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

    let flags = crate::registry::get_flags();
    let registered_functions = get_registered_functions(&flags);

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

fn get_registered_functions(flags: &[Flag]) -> Vec<RegisteredHeatFunction> {
    flags
        .iter()
        .map(|flag| {
            // function token stream to readable string
            let itemfn = syn_serde::json::from_slice::<syn::ItemFn>(flag.token_stream)
                .expect("Should be able to parse token stream.");
            let syn_tree: syn::File = syn::parse2(itemfn.into_token_stream())
                .expect("Should be able to parse token stream.");
            let code_str = prettyplease::unparse(&syn_tree);
            RegisteredHeatFunction {
                mod_path: flag.mod_path.to_string(),
                fn_name: flag.fn_name.to_string(),
                proc_type: flag.proc_type.to_string(),
                code: code_str,
            }
        })
        .collect()
}
