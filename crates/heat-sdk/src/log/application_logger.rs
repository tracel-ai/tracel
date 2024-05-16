use crate::client::HeatClientState;
use burn::train::ApplicationLoggerInstaller;
use tracing_subscriber::fmt::MakeWriter;

use tracing_core::{Level, LevelFilter};
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{registry, Layer};

pub struct RemoteExperimentLoggerInstaller {
    client: HeatClientState,
}

impl RemoteExperimentLoggerInstaller {
    pub fn new(client: HeatClientState) -> Self {
        Self { client }
    }
}

struct RemoteWriter {
    client: HeatClientState,
}

struct RemoteWriterMaker {
    client: HeatClientState,
}

impl<'a> MakeWriter<'a> for RemoteWriterMaker {
    type Writer = RemoteWriter;

    fn make_writer(&self) -> Self::Writer {
        RemoteWriter {
            client: self.client.clone(),
        }
    }
}

impl std::io::Write for RemoteWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let message = String::from_utf8_lossy(buf).to_string();
        let mut client = self
            .client
            .lock()
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))?;

        client
            .log_experiment(message)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl ApplicationLoggerInstaller for RemoteExperimentLoggerInstaller {
    fn install(&self) -> Result<(), String> {
        let make_writer = RemoteWriterMaker {
            client: self.client.clone(),
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
            log::error!("PANIC => {}", info.to_string());
            eprintln!(
                "=== PANIC ===\nA fatal error happened, you can check the experiment logs on Heat.\n============="
            );
            hook(info);
        }));

        Ok(())
    }
}
