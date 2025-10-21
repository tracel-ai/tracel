use std::collections::HashMap;

use burn::train::logger::MetricLogger;
use burn::train::metric::{MetricEntry, NumericEntry};

use crate::experiment::{ExperimentRun, ExperimentRunHandle};

/// The remote metric logger, used to send metric logs to Burn Central.
pub struct RemoteMetricLogger {
    experiment_handle: ExperimentRunHandle,
    epoch: usize,
    iterations: HashMap<String, usize>,
    group: String,
}

impl RemoteMetricLogger {
    /// Create a new instance of the remote metric logger with the given [BurnCentralClientState] and metric group name.
    pub fn new(experiment: &ExperimentRun, group: String) -> Self {
        Self {
            experiment_handle: experiment.handle(),
            epoch: 1,
            iterations: HashMap::new(),
            group,
        }
    }
}

impl MetricLogger for RemoteMetricLogger {
    fn log(&mut self, item: &MetricEntry) {
        let key = &item.name;
        let value = &item.serialize;
        // deserialize
        let numeric_entry: NumericEntry = match NumericEntry::deserialize(value) {
            Ok(v) => v,
            Err(_) => return,
        };

        let iteration = self.iterations.entry(key.to_string()).or_insert(0);

        // send to server
        self.experiment_handle.log_metric(
            key.to_string(),
            self.epoch,
            *iteration,
            match numeric_entry {
                NumericEntry::Value(v) => v,
                NumericEntry::Aggregated { sum: v, .. } => v,
            },
            self.group.clone(),
        );

        // todo: this is an incorrect way to get the iteration, ideally, the learner would provide this on every log call.
        *iteration += 1;
    }

    fn end_epoch(&mut self, epoch: usize) {
        self.epoch = epoch + 1;
    }

    /// Read the logs for an epoch.
    fn read_numeric(&mut self, _name: &str, _epoch: usize) -> Result<Vec<NumericEntry>, String> {
        Ok(vec![]) // Not implemented
    }
}
