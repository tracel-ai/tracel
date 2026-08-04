//! End-to-end smoke test: create a Cloud project, then run a trivial experiment in it.
//!
//! cargo run -p create_project_smoke -- <owner> <name> [description]
//!
//! Run from this directory so `tracel.toml` is read/written here. Needs `tracel login` (or
//! `TRACEL_API_KEY`) done beforehand.

use clap::Parser;
use tracel::experiment::ExperimentRun;
use tracel::{Connection, Context};

#[derive(Parser)]
struct Args {
    owner: String,
    name: String,
    #[arg(default_value = "smoke test project")]
    description: String,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    Connection::create_project(&args.owner, &args.name, &args.description)?;
    println!("project '{}/{}' created", args.owner, args.name);

    let context = Context::new(Connection::Cloud)?;

    context
        .experiment()
        .create("smoke-test", |run: &ExperimentRun, _: ()| {
            run.log_args(&())?;
            Ok(())
        })
        .run(())
        .map_err(|e| anyhow::anyhow!("experiment run failed: {e}"))?;

    println!("experiment run completed");
    Ok(())
}
