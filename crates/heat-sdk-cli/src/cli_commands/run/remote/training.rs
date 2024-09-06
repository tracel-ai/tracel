use clap::Parser;
use heat_sdk::{
    client::{HeatClient, HeatClientConfig, HeatCredentials},
    schemas::{HeatCodeMetadata, ProjectPath, RegisteredHeatFunction},
};
use quote::ToTokens;

use crate::{context::HeatCliContext, generation::backend::BackendType};

/// Run a training remotely.
/// Not yet supported.
#[derive(Parser, Debug)]
pub struct RemoteTrainingRunArgs {
    /// The training functions to run
    #[clap(short = 'f', long="functions", value_delimiter = ' ', num_args = 1.., required = true, help = "<required> The training functions to run. Annotate a training function with #[heat(training)] to register it.")]
    functions: Vec<String>,
    /// Backend to use
    #[clap(short = 'b', long = "backends", value_delimiter = ' ', num_args = 1.., required = true, help = "<required> Backends to use for training.")]
    backends: Vec<BackendType>,
    /// Config files paths
    #[clap(short = 'c', long = "configs", value_delimiter = ' ', num_args = 1.., required = true, help = "<required> Config files paths.")]
    configs: Vec<String>,
    /// The Heat project ID
    // todo: support project name and creating a project if it doesn't exist
    #[clap(
        short = 'p',
        long = "project",
        required = true,
        help = "<required> The Heat project ID."
    )]
    project_path: String,
    /// The Heat API key
    #[clap(
        short = 'k',
        long = "key",
        required = true,
        help = "<required> The Heat API key."
    )]
    key: String,
    /// The Heat API endpoint
    #[clap(
        short = 'e',
        long = "endpoint",
        help = "The Heat API enpoint.",
        default_value = "http://127.0.0.1:9001"
    )]
    pub heat_endpoint: String,
    /// The runner group name
    #[clap(
        short = 'r',
        long = "runner",
        help = "The runner group name.",
        required = true
    )]
    pub runner: String,
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

pub(crate) fn handle_command(
    args: RemoteTrainingRunArgs,
    context: HeatCliContext,
) -> anyhow::Result<()> {
    let heat_client = create_heat_client(&args.key, &args.heat_endpoint, &args.project_path);

    let crates = crate::util::cargo::package::package(
        &context.get_artifacts_dir_path(),
        context.package_name(),
    )?;

    let flags = crate::registry::get_flags();

    let mut registered_functions = Vec::<RegisteredHeatFunction>::new();
    for flag in flags {
        // function token stream to readable string
        let itemfn = syn_serde::json::from_slice::<syn::ItemFn>(flag.token_stream)
            .expect("Should be able to parse token stream.");
        let syn_tree: syn::File =
            syn::parse2(itemfn.into_token_stream()).expect("Should be able to parse token stream.");
        let code_str = prettyplease::unparse(&syn_tree);
        registered_functions.push(RegisteredHeatFunction {
            mod_path: flag.mod_path.to_string(),
            fn_name: flag.fn_name.to_string(),
            proc_type: flag.proc_type.to_string(),
            code: code_str,
        });
    }

    let heat_metadata = HeatCodeMetadata {
        functions: registered_functions,
    };

    let project_version =
        heat_client.upload_new_project_version(context.package_name(), heat_metadata, crates)?;

    heat_client.start_remote_job(
        args.runner,
        project_version,
        format!(
            "run local training --functions {} --backends {} --configs {} --project {} --key {}",
            args.functions.join(" "),
            args.backends
                .into_iter()
                .map(|backend| backend.to_string())
                .collect::<Vec<_>>()
                .join(" "),
            args.configs.join(" "),
            args.project_path,
            args.key
        ),
    )?;

    Ok(())
}
