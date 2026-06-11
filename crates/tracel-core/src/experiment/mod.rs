mod local;
mod remote;

// TODO: temporary re-export for the runtime crate, will be erased when we detach ourself completely from runtime
pub use remote::cloud::create_cloud_experiment_run;

pub use tracel_experiment::{ExperimentFn, ExperimentJob, ExperimentModule, ExperimentProvider};
