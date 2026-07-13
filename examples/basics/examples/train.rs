//! An experiment run: a toy training loop with activity tracking, metrics, and cancellation.
//!
//! cargo run -p basics --example train

use basics::training::{self, TrainingConfig};
use tracel::experiment::ExperimentRun;

fn main() -> anyhow::Result<()> {
    let module = common::context()?.experiment();

    module
        .create("toy-training", |run: &ExperimentRun, config| {
            training::train(run, config)
        })
        .attribute("kind", "example")?
        .run(TrainingConfig::default())
        .map_err(|e| anyhow::anyhow!("training failed: {e}"))?;

    Ok(())
}
