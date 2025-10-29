use std::collections::HashMap;

use burn::train::logger::MetricLogger;
use burn::train::metric::store::Split;
use burn::train::metric::{MetricAttributes, MetricEntry, NumericEntry};

use crate::experiment::{ExperimentRun, ExperimentRunHandle};

/// The remote metric logger, used to send metric logs to Burn Central.
pub struct RemoteMetricLogger {
    experiment_handle: ExperimentRunHandle,
    iterations: HashMap<String, usize>,
}

impl RemoteMetricLogger {
    /// Create a new instance of the remote metric logger with the given [BurnCentralClientState] and metric group name.
    pub fn new(experiment: &ExperimentRun) -> Self {
        Self {
            experiment_handle: experiment.handle(),
            iterations: HashMap::new(),
        }
    }
}

impl MetricLogger for RemoteMetricLogger {
    fn log(&mut self, item: &MetricEntry, epoch: usize, split: Split) {
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
            epoch,
            *iteration,
            match numeric_entry {
                NumericEntry::Value(v) => v,
                NumericEntry::Aggregated { sum: v, .. } => v,
            },
            split.to_string(),
        );

        // todo: this is an incorrect way to get the iteration, ideally, the learner would provide this on every log call.
        *iteration += 1;
    }

    /// Read the logs for an epoch.
    fn read_numeric(
        &mut self,
        _name: &str,
        _epoch: usize,
        _split: Split,
    ) -> Result<Vec<NumericEntry>, String> {
        Ok(vec![]) // Not implemented
    }

    fn log_metric_definition(&self, definition: burn::train::metric::MetricDefinition) {
        let (unit, higher_is_better) = match &definition.attributes {
            MetricAttributes::Numeric(attr) => (attr.unit.clone(), attr.higher_is_better),
            MetricAttributes::None => return,
        };

        match self.experiment_handle.log_metric_definition(
            definition.name,
            definition.description,
            unit,
            higher_is_better,
        ) {
            Ok(_) => (),
            Err(e) => panic!("{e}"),
        }
    }
}
