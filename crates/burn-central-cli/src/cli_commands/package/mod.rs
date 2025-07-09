use crate::context::CliContext;
use crate::print_success;
use crate::registry::Flag;
use crate::util::git::get_last_commit_hash;
use burn_central_client::schemas::{BurnCentralCodeMetadata, RegisteredFunction};
use clap::Args;
use quote::ToTokens;

#[derive(Args, Debug)]
pub struct PackageArgs {}

pub(crate) fn handle_command(_args: PackageArgs, context: CliContext) -> anyhow::Result<()> {
    let last_commit_hash = get_last_commit_hash()?;

    let client = context.create_client()?;
    let crates = crate::util::cargo::package::package(
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
        &project_path.owner_name(),
        &project_path.project_name(),
        context.package_name(),
        code_metadata,
        crates,
        &last_commit_hash,
    )?;

    print_success!("New project version uploaded: {}", project_version);

    Ok(())
}

fn get_registered_functions(flags: &[Flag]) -> Vec<RegisteredFunction> {
    flags
        .iter()
        .map(|flag| {
            // function token stream to readable string
            let itemfn = syn_serde::json::from_slice::<syn::ItemFn>(flag.token_stream)
                .expect("Should be able to parse token stream.");
            let syn_tree: syn::File = syn::parse2(itemfn.into_token_stream())
                .expect("Should be able to parse token stream.");
            let code_str = prettyplease::unparse(&syn_tree);
            RegisteredFunction {
                mod_path: flag.mod_path.to_string(),
                fn_name: flag.fn_name.to_string(),
                proc_type: flag.proc_type.to_string(),
                code: code_str,
            }
        })
        .collect()
}
