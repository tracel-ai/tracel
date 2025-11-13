use std::collections::HashMap;

use burn::train::logger::MetricLogger;
use burn::train::metric::store::{EpochSummary, Split};
use burn::train::metric::{MetricAttributes, MetricDefinition, MetricEntry, NumericEntry};
use derive_new::new;
use serde::Serialize;

use crate::experiment::{ExperimentRun, ExperimentRunHandle};

#[derive(Debug, Serialize, new)]
pub struct MetricLog {
    name: String,
    value: f64,
}

/// The remote metric logger, used to send metric logs to Burn Central.
pub struct RemoteMetricLogger {
    experiment_handle: ExperimentRunHandle,
    iteration_count: usize,
}

impl RemoteMetricLogger {
    /// Create a new instance of the remote metric logger with the given [BurnCentralClientState] and metric group name.
    pub fn new(experiment: &ExperimentRun) -> Self {
        Self {
            experiment_handle: experiment.handle(),
            iteration_count: 0,
        }
    }
}

impl MetricLogger for RemoteMetricLogger {
    fn log(&mut self, items: Vec<&MetricEntry>, epoch: usize, split: Split) {
        self.iteration_count += 1;
        let item_logs: Vec<MetricLog> = items
            .iter()
            .filter_map(|entry| {
                let numeric_entry: NumericEntry = match NumericEntry::deserialize(&entry.serialize)
                {
                    Ok(e) => e,
                    Err(_) => return None,
                };
                let value = match numeric_entry {
                    NumericEntry::Value(v) => v,
                    NumericEntry::Aggregated {
                        aggregated_value, ..
                    } => aggregated_value,
                };
                Some(MetricLog::new(entry.name.to_string(), value))
            })
            .collect();

        if item_logs.is_empty() {
            return;
        };

        // send to server
        self.experiment_handle.log_metric(
            epoch,
            split.to_string(),
            self.iteration_count,
            item_logs,
        );
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

    fn log_metric_definition(&self, definition: MetricDefinition) {
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

    fn log_epoch_summary(&mut self, summary: EpochSummary) {
        let best_metric_values: HashMap<String, f64> = summary
            .clone()
            .best_metric_values
            .into_iter()
            .filter_map(|(k, v)| v.map(|val| (k, val.current())))
            .collect();
        match self.experiment_handle.log_epoch_summary(
            summary.epoch_number,
            summary.split.to_string(),
            best_metric_values,
        ) {
            Ok(_) => (),
            Err(e) => panic!("{e}"),
        }
    }
}
