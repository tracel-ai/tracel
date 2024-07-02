use clap::Parser;
use guide_cli::guide_mod::__heat_main::*;

#[derive(Parser, Debug)]
pub struct Args {
    #[clap(short = 'c', long = "configs", value_delimiter = ' ', num_args = 1.., required = true)]
    config_paths: Vec<String>,
}

fn main() {
    println!("Running bin.");
    let args = Args::parse();
    for config_path in &args.config_paths {
        heat_main(&config_path);
    }
}
