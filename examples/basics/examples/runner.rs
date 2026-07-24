//! A station runner serving the training job to a Tracel Station job queue.
//!
//! TRACEL_CONNECTION=station cargo run -p basics --example runner --features station
//!
//! Then queue, watch, and cancel jobs through the station API:
//! curl -X POST localhost:8000/v1/jobs -H 'content-type: application/json' \
//!     -d '{"job_name":"toy-training","input":{"epochs":2}}'
//! curl localhost:8000/v1/jobs
//! curl -X PUT localhost:8000/v1/jobs/<job_id>/cancel

use basics::training::{self, TrainingConfig};
use tracel::experiment::ExperimentRun;
use tracel::runner::StationRunner;
use tracel::runner::mapper::JsonInput;

fn main() -> anyhow::Result<()> {
    let context = common::context()?;

    let train = context
        .experiment()
        .create("toy-training", |run: &ExperimentRun, config| {
            training::train(run, config)
        });

    StationRunner::new(common::station_url()?.as_str())
        .name("basics-runner")
        .register(train, JsonInput::with_default(TrainingConfig::default()))
        .run()?;

    Ok(())
}
