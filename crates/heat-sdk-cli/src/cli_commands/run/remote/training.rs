use clap::Parser;
use heat_sdk::{
    client::{HeatClient, HeatClientConfig, HeatCredentials},
    schemas::CrateData,
};

use crate::{context::HeatCliContext, generation::backend::BackendType, package::PackagedCrates};

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
    project: String,
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
}

fn create_heat_client(api_key: &str, url: &str, project: &str) -> HeatClient {
    let creds = HeatCredentials::new(api_key.to_owned());
    let client_config = HeatClientConfig::builder(creds, project)
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
    let heat_client = create_heat_client(&args.key, &args.heat_endpoint, &args.project);

    let PackagedCrates {
        root_package_name,
        mut crates,
    } = crate::package::package(&context)?;

    println!("Uploading crates to Heat server...");
    let mut crates_data: Vec<CrateData> = Vec::new();
    for dst in crates.drain(..) {
        let metadata = dst.metadata;
        let data = std::fs::read(dst.path)?;
        crates_data.push(CrateData { metadata, data });
    }

    let project_version = heat_client.upload_crates(&root_package_name, crates_data)?;

    heat_client.start_remote_job(
        project_version,
        root_package_name,
        format!(
            "run local training --functions {} --backends {} --configs {} --project {} --key {}",
            args.functions.join(" "),
            args.backends
                .into_iter()
                .map(|backend| backend.to_string())
                .collect::<Vec<_>>()
                .join(" "),
            args.configs.join(" "),
            args.project,
            args.key
        ),
    )?;

    Ok(())
}
