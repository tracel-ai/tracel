//! An experiment run: a toy training loop with activity tracking, metrics, and cancellation.
//!
//! cargo run -p basics --example train

use basics::training::{self, TrainingConfig};
use tracel::Target;

fn main() -> anyhow::Result<()> {
    let experiments = Target::from_env()?.experiments()?;
    let config = TrainingConfig::default();

    experiments
        .attribute("kind", "example")?
        .run("toy-training", |run| {
            run.log_args(&config)?;
            training::train(run, config)
        })
        .map_err(|e| anyhow::anyhow!("training failed: {e}"))
}
