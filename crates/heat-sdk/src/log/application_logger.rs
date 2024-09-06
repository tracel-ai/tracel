use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::client::HeatClientState;
use crate::experiment::WsMessage;
use burn::train::ApplicationLoggerInstaller;
use tracing_subscriber::fmt::MakeWriter;

use tracing_core::{Level, LevelFilter};
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{registry, Layer};

/// The installer for the remote experiment logger.
pub struct RemoteExperimentLoggerInstaller {
    client: Arc<Mutex<HeatClientState>>,
}

impl RemoteExperimentLoggerInstaller {
    /// Creates a new instance of the remote experiment logger installer with the given [HeatClientState].
    pub fn new(client: HeatClientState) -> Self {
        Self {
            client: Arc::new(Mutex::new(client)),
        }
    }
}

struct RemoteWriter {
    sender: Option<Sender<WsMessage>>,
}

struct RemoteWriterMaker {
    client: HeatClientState,
}

impl<'a> MakeWriter<'a> for RemoteWriterMaker {
    type Writer = RemoteWriter;

    fn make_writer(&self) -> Self::Writer {
        if let Ok(sender) = self.client.get_experiment_sender() {
            RemoteWriter {
                sender: Some(sender),
            }
        } else {
            RemoteWriter { sender: None }
        }
    }
}

impl std::io::Write for RemoteWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let message = String::from_utf8_lossy(buf).to_string();

        if let Some(sender) = &self.sender {
            sender.send(WsMessage::Log(message)).unwrap();
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl ApplicationLoggerInstaller for RemoteExperimentLoggerInstaller {
    fn install(&self) -> Result<(), String> {
        let make_writer = {
            let heat_client = self.client.lock().unwrap();
            RemoteWriterMaker {
                client: heat_client.clone(),
            }
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
        let client = Arc::downgrade(&self.client);
        std::panic::set_hook(Box::new(move |info| {
            log::error!("PANIC => {}", info.to_string());
            eprintln!(
                "=== PANIC ===\nA fatal error happened, you can check the experiment logs on Heat.\n============="
            );
            if let Some(client) = client.upgrade().as_mut() {
                let mut guard = client.lock().unwrap();
                guard
                    .end_experiment_with_error("Panic".to_string())
                    .expect("Should end the experiment");
            }

            hook(info);
        }));

        Ok(())
    }
}
