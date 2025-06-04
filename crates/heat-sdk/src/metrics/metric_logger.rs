use std::sync::mpsc;

use burn::train::logger::MetricLogger;
use burn::train::metric::{MetricEntry, NumericEntry};

use crate::client::BurnCentralClientState;
use crate::errors::client::BurnCentralClientError;
use crate::experiment::{Split, WsMessage};

/// The remote metric logger, used to send metric logs to Burn Central.
pub struct RemoteMetricLogger {
    sender: mpsc::Sender<WsMessage>,
    epoch: usize,
    split: Split,
}

impl RemoteMetricLogger {
    /// Create a new instance of the remote metric logger for `Training` with the given [BurnCentralClientState].
    pub fn new_train(client: BurnCentralClientState) -> Self {
        Self::new(client, Split::Train)
            .expect("RemoteMetricLogger should be created successfully for training split")
    }

    /// Create a new instance of the remote metric logger for `Validation` with the given [BurnCentralClientState].
    pub fn new_validation(client: BurnCentralClientState) -> Self {
        Self::new(client, Split::Val)
            .expect("RemoteMetricLogger should be created successfully for validation split")
    }

    fn new(client: BurnCentralClientState, split: Split) -> Result<Self, BurnCentralClientError> {
        Ok(Self {
            sender: client
                .get_experiment_sender()
                .map_err(|e| BurnCentralClientError::CreateRemoteMetricLoggerError(e.to_string()))?,
            epoch: 1,
            split,
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

        // send to server
        self.sender
            .send(WsMessage::MetricLog {
                name: key.clone(),
                epoch: self.epoch,
                value: match numeric_entry {
                    NumericEntry::Value(v) => v,
                    NumericEntry::Aggregated(v, _) => v,
                },
                split: self.split.clone(),
            })
            .unwrap();
    }

    fn end_epoch(&mut self, epoch: usize) {
        self.epoch = epoch + 1;
    }

    /// Read the logs for an epoch.
    fn read_numeric(&mut self, _name: &str, _epoch: usize) -> Result<Vec<NumericEntry>, String> {
        Ok(vec![]) // Not implemented
    }
}
