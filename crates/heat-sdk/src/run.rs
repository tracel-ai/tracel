use clap::Parser;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct RunArgs {
    /// Config file path
    #[clap(short = 'c', long = "configs", value_delimiter = ' ', num_args = 1.., required = true)]
    configs: Vec<String>,
    /// The project ID
    // todo: support project name and creating a project if it doesn't exist
    #[clap(short = 'p', long = "project", required = true)]
    project: String,
    /// The API key
    #[clap(short = 'k', long = "key", required = true)]
    key: String,
    /// The Heat endpoint
    #[clap(
        short = 'e',
        long = "heat-endpoint",
        default_value = "http://127.0.0.1:9001"
    )]
    heat_endpoint: String,
}

pub struct RunConfig {
    pub configs_paths: Vec<String>,
    pub project: String,
    pub key: String,
    pub heat_endpoint: String,
}

pub fn get_run_config() -> RunConfig {
    let args = RunArgs::parse();
    RunConfig {
        configs_paths: args.configs,
        project: args.project,
        key: args.key,
        heat_endpoint: args.heat_endpoint,
    }
}
