use crate::context::HeatCliContext;
use crate::registry::Flag;
use crate::{print_err, print_success};
use clap::Parser;
use burn_central_client::client::{HeatClient, HeatClientConfig, HeatCredentials};
use burn_central_client::schemas::{HeatCodeMetadata, ProjectPath, RegisteredHeatFunction};
use quote::ToTokens;

#[derive(Parser, Debug)]
pub struct PackageArgs {
    /// The Heat project path
    // todo: support project name and creating a project if it doesn't exist
    #[clap(
        short = 'p',
        long = "project",
        required = true,
        help = "The Heat project path. Ex: test/Default-Project"
    )]
    project_path: String,
    /// The Heat API key
    #[clap(short = 'k', long = "key", required = true, help = "The Heat API key.")]
    key: String,
}

pub(crate) fn handle_command(args: PackageArgs, context: HeatCliContext) -> anyhow::Result<()> {
    let last_commit_hash = get_last_commit_hash()?;

    let heat_client = create_heat_client(
        &args.key,
        context.get_api_endpoint().as_str(),
        &args.project_path,
    );

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

fn create_heat_client(api_key: &str, url: &str, project_path: &str) -> HeatClient {
    let creds = HeatCredentials::new(api_key.to_owned());
    let client_config = HeatClientConfig::builder(
        creds,
        ProjectPath::try_from(project_path.to_string()).expect("Project path should be valid."),
    )
    .with_endpoint(url)
    .with_num_retries(10)
    .build();
    HeatClient::create(client_config)
        .expect("Should connect to the Heat server and create a client")
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

fn get_last_commit_hash() -> anyhow::Result<String> {
    let repo = gix::discover(".")?;
    let last_commit = repo.head()?.peel_to_commit_in_place()?.id();
    if repo.is_dirty()? {
        print_err!("Latest git commit: {}", last_commit);
        anyhow::bail!("Repo is dirty. Please commit or stash your changes before packaging.");
    }

    Ok(last_commit.to_string())
}
