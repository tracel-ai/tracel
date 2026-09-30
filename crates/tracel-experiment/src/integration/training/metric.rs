use std::collections::HashMap;

use burn::train::logger::MetricLogger;
use burn::train::metric::store::{EpochSummary, MetricsUpdate, Split};
use burn::train::metric::{MetricAttributes, MetricDefinition, MetricId, NumericEntry};

use crate::{ExperimentRunHandle, MetricSpec, MetricValue};

/// Experiment-backed implementation of Burn's [`MetricLogger`] trait.
///
/// Prefer [`crate::integration::training::ExperimentTrainingExt::metric_logger`] when you already
/// have an [`ExperimentRun`][crate::ExperimentRun] in scope.
pub struct ExperimentMetricLogger {
    experiment_handle: ExperimentRunHandle,
    metric_definitions: HashMap<MetricId, MetricDefinition>,
    iteration_count: usize,
    epoch_final_values_awaiting_summary: HashMap<Split, Vec<MetricValue>>,
}

impl ExperimentMetricLogger {
    /// Create a metric logger backed by the provided experiment run.
    pub fn new(experiment: impl Into<ExperimentRunHandle>) -> Self {
        Self {
            experiment_handle: experiment.into(),
            metric_definitions: HashMap::default(),
            iteration_count: 0,
            epoch_final_values_awaiting_summary: HashMap::default(),
        }
    }
}

impl MetricLogger for ExperimentMetricLogger {
    fn log(&mut self, update: MetricsUpdate, epoch: usize, split: &Split) {
        let mut iteration_values = vec![];
        for numeric_update in &update.entries_numeric {
            let Some(definition) = self.metric_definitions.get(&numeric_update.entry.metric_id)
            else {
                continue;
            };
            let Some(numeric_entry) = &numeric_update.numeric_entry else {
                continue;
            };
            let name = definition.name.to_string();
            match *numeric_entry {
                NumericEntry::Final(value) => self
                    .epoch_final_values_awaiting_summary
                    .entry(split.clone())
                    .or_default()
                    .push(MetricValue { name, value }),
                NumericEntry::Value(value)
                | NumericEntry::Aggregated {
                    aggregated_value: value,
                    ..
                } => iteration_values.push(MetricValue { name, value }),
            }
        }

        if iteration_values.is_empty() {
            return;
        }
        self.iteration_count += 1;
        self.experiment_handle.log_metric(
            epoch,
            split.to_string(),
            self.iteration_count,
            iteration_values,
        );
    }

    /// Read the logs for an epoch.
    fn read_numeric(
        &mut self,
        _name: &str,
        _epoch: usize,
        _split: &Split,
    ) -> Result<Vec<NumericEntry>, String> {
        Ok(vec![]) // Not implemented
    }

    fn log_metric_definition(&mut self, definition: burn::train::metric::MetricDefinition) {
        self.metric_definitions
            .insert(definition.metric_id.clone(), definition.clone());

        let (unit, higher_is_better) = match &definition.attributes {
            MetricAttributes::Numeric(attr) => (attr.unit.clone(), attr.higher_is_better),
            MetricAttributes::None => return,
        };

        self.experiment_handle.log_metric_definition(MetricSpec {
            name: definition.name.to_string(),
            description: definition.description,
            unit,
            higher_is_better,
        });
    }

    fn log_epoch_summary(&mut self, summary: EpochSummary) {
        if let Some(final_values) = self
            .epoch_final_values_awaiting_summary
            .remove(&summary.split)
        {
            self.experiment_handle.log_epoch_summary(
                summary.epoch_number,
                summary.split.to_string(),
                final_values,
            );
        }
    }
}
