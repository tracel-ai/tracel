//! A station runner serving the training job to a Tracel Station job queue.
//!
//! TRACEL_TARGET=station cargo run -p basics --example runner --features station
//!
//! Then queue, watch, and cancel jobs through the station API:
//! curl -X POST localhost:8000/v1/jobs -H 'content-type: application/json' \
//!     -d '{"job_name":"toy-training","input":{"epochs":2}}'
//! curl localhost:8000/v1/jobs
//! curl -X PUT localhost:8000/v1/jobs/<job_id>/cancel

use basics::training::{self, TrainingConfig};
use tracel::Target;
use tracel::runtime::station::StationRunner;
use tracel::runtime::{BoxError, Job, Jobs};

/// Runs the toy training loop in an experiment, which records the job's input as its arguments.
fn toy_training(config: TrainingConfig) -> Result<(), BoxError> {
    let experiments = Target::from_env()?.experiments()?;
    experiments.run("toy-training", |run| training::train(run, config))
}

fn main() -> anyhow::Result<()> {
    let Target::Station { url } = Target::from_env()? else {
        anyhow::bail!("set TRACEL_TARGET=station to serve jobs to a Tracel Station");
    };

    let jobs = Jobs::new().job(
        "toy-training",
        Job::new(toy_training)
            .with_description("Run a toy training loop")
            .with_default(TrainingConfig::default())
            .with_schema(),
    );
    StationRunner::new(url.as_str())
        .name("basics-runner")
        .run(jobs)?;

    Ok(())
}
