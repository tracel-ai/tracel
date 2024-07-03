use clap::Parser;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct RunArgs {
    /// Config file path
    #[clap(short = 'c', long = "configs", value_delimiter = ' ', num_args = 1.., required = true)]
    configs: Vec<String>,
}

pub struct RunConfig {
    pub configs_paths: Vec<String>,
}

pub fn get_run_config() -> RunConfig {
    let args = RunArgs::parse();
    RunConfig {
        configs_paths: args.configs,
    }
}
