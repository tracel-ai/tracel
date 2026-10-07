//! An experiment run: a toy training loop with activity tracking, metrics, and cancellation.
//!
//! cargo run -p basics --example train

use basics::training::{self, TrainingConfig};
use tracel::Target;

fn main() -> anyhow::Result<()> {
    let experiments = Target::from_env()?.experiments()?;

    experiments
        .create("toy-training", training::train)
        .attribute("kind", "example")?
        .run(TrainingConfig::default())
        .map_err(|e| anyhow::anyhow!("training failed: {e}"))?;

    Ok(())
}
