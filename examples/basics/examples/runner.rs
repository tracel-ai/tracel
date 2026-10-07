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
use tracel::Target;
use tracel::app::mapper::JsonMapper;
use tracel::runner::StationRunner;

fn main() -> anyhow::Result<()> {
    let target = Target::from_env()?;
    let Target::Station { url } = &target else {
        anyhow::bail!("set TRACEL_CONNECTION=station to serve jobs to a Tracel Station");
    };

    let train = target
        .experiments()?
        .create("toy-training", training::train)
        .with_description("Run a toy training loop");

    StationRunner::new(url.as_str())
        .name("basics-runner")
        .register(
            train,
            JsonMapper::with_default(TrainingConfig::default()).with_schema(),
        )
        .run()?;

    Ok(())
}
