use std::collections::HashMap;
use std::sync::mpsc;

use burn::train::logger::MetricLogger;
use burn::train::metric::{MetricEntry, NumericEntry};

use crate::client::BurnCentralClientState;
use crate::error::BurnCentralClientError;
use crate::experiment::{Experiment, ExperimentHandle, ExperimentMessage};

/// The remote metric logger, used to send metric logs to Burn Central.
pub struct RemoteMetricLogger {
    experiment_handle: ExperimentHandle,
    epoch: usize,
    iterations: HashMap<String, usize>,
    group: String,
}

impl RemoteMetricLogger {
    /// Create a new instance of the remote metric logger with the given [BurnCentralClientState] and metric group name.
    pub fn new(
        experiment_handle: &Experiment,
        group: String,
    ) -> Result<Self, BurnCentralClientError> {
        Ok(Self {
            experiment_handle: experiment_handle.handle(),
            epoch: 1,
            iterations: HashMap::new(),
            group,
        })
    }
}

fn deserialize_numeric_entry(entry: &str) -> Result<NumericEntry, String> {
    // Check for comma separated values
    let values = entry.split(',').collect::<Vec<_>>();
    let num_values = values.len();

    if num_values == 1 {
        // Numeric value
        match values[0].parse::<f64>() {
            Ok(value) => Ok(NumericEntry::Value(value)),
            Err(err) => Err(err.to_string()),
        }
    } else if num_values == 2 {
        // Aggregated numeric (value, number of elements)
        let (value, numel) = (values[0], values[1]);
        match value.parse::<f64>() {
            Ok(value) => match numel.parse::<usize>() {
                Ok(numel) => Ok(NumericEntry::Aggregated(value, numel)),
                Err(err) => Err(err.to_string()),
            },
            Err(err) => Err(err.to_string()),
        }
    } else {
        Err("Invalid number of values for numeric entry".to_string())
    }
}

impl MetricLogger for RemoteMetricLogger {
    fn log(&mut self, item: &MetricEntry) {
        let key = &item.name;
        let value = &item.serialize;

        // deserialize
        let numeric_entry: NumericEntry = deserialize_numeric_entry(value).unwrap();

        let iteration = self.iterations.entry(key.clone()).or_insert(0);

        // send to server
        self.experiment_handle.log_metric(
            key.clone(),
            self.epoch,
            *iteration,
            match numeric_entry {
                NumericEntry::Value(v) => v,
                NumericEntry::Aggregated(v, _) => v,
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
