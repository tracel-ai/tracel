use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::client::BurnCentralClientState;
use crate::experiment::{Experiment, ExperimentHandle, ExperimentMessage};
use burn::train::ApplicationLoggerInstaller;
use tracing_subscriber::fmt::MakeWriter;

use tracing_core::{Level, LevelFilter};
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{Layer, registry};

/// The installer for the remote experiment logger.
pub struct RemoteExperimentLoggerInstaller {
    experiment_handle: Arc<ExperimentHandle>,
}

impl RemoteExperimentLoggerInstaller {
    /// Creates a new instance of the remote experiment logger installer with the given [BurnCentralClientState].
    pub fn new(experiment_handle: &Experiment) -> Self {
        Self {
            experiment_handle: Arc::new(experiment_handle.handle()),
        }
    }
}

struct RemoteWriter {
    sender: Arc<ExperimentHandle>,
}

struct RemoteWriterMaker {
    experiment_handle: Arc<ExperimentHandle>,
}

impl MakeWriter<'_> for RemoteWriterMaker {
    type Writer = RemoteWriter;

    fn make_writer(&self) -> Self::Writer {
        let sender = self.experiment_handle.clone();
        RemoteWriter { sender }
    }
}

impl std::io::Write for RemoteWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let message = String::from_utf8_lossy(buf).to_string();

        self.sender.log_info(message);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl ApplicationLoggerInstaller for RemoteExperimentLoggerInstaller {
    fn install(&self) -> Result<(), String> {
        let make_writer = RemoteWriterMaker {
            experiment_handle: self.experiment_handle.clone(),
        };

        let layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(make_writer)
            .with_filter(LevelFilter::INFO)
            .with_filter(filter_fn(|m| {
                if let Some(path) = m.module_path() {
                    // The wgpu crate is logging too much, so we skip `info` level.
                    if path.starts_with("wgpu") && *m.level() >= Level::INFO {
                        return false;
                    }
                }
                true
            }));

        if registry().with(layer).try_init().is_err() {
            return Err("Failed to install the file logger.".to_string());
        }

        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log::error!("PANIC => {info}");
            eprintln!(
                "=== PANIC ===\nA fatal error happened, you can check the experiment logs on Burn Central.\n============="
            );
            hook(info);
        }));

        Ok(())
    }
}
