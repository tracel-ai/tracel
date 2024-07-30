use crate::{context::HeatCliContext, package::PackagedCrates};
use clap::Parser;
use heat_sdk::{
    client::{HeatClient, HeatClientConfig, HeatCredentials},
    schemas::CrateData,
};

#[derive(Parser, Debug)]
pub struct PackageArgs {
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

pub(crate) fn handle_command(args: PackageArgs, context: HeatCliContext) -> anyhow::Result<()> {
    let heat_client = create_heat_client(&args.key, &args.heat_endpoint, &args.project);

    let PackagedCrates {
        root_package_name,
        mut crates,
    } = crate::package::package(&context)?;

    let mut crates_data: Vec<CrateData> = Vec::new();
    for dst in crates.drain(..) {
        let metadata = dst.metadata;
        let data = std::fs::read(dst.path)?;
        crates_data.push(CrateData { metadata, data });
    }

    heat_client.upload_crates(&root_package_name, crates_data)?;

    Ok(())
}
