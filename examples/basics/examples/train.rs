//! Programmatic experiment run: a toy training loop that tracks nested activities (with progress
//! and cancellation), declares its metrics, and logs per-batch/-epoch values.
//!
//! Run: `cargo run -p basics --example train`

use basics::training::{self, TrainingConfig};
use tracel::experiment::ExperimentRun;
use tracel::{Connection, Context};

fn main() -> anyhow::Result<()> {
    // For Cloud (so activities and metrics ship to the dashboard):
    //   let module = Context::new(Connection::Cloud)?.experiment();
    let module = Context::new(Connection::Offline("./runs".into()))?.experiment();

    module
        .create("toy-training", |run: &ExperimentRun, config| {
            training::train(run, config)
        })
        .attribute("kind", "example")?
        .run(TrainingConfig::default())
        .map_err(|e| anyhow::anyhow!("training failed: {e}"))?;

    Ok(())
}
