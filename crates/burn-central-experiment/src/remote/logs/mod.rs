use burn_central_artifact::bundle::InMemoryBundleSources;

mod uploader;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct LogStoreError {
    message: String,
    #[source]
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl LogStoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source<E>(message: impl Into<String>, source: E) -> Self
    where
        E: Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    {
        Self {
            message: message.into(),
            source: Some(source.into()),
        }
    }
}

pub struct TempLogStore {
    logs: Vec<String>,
    bytes: usize,
    artifact_id: Option<String>,
    file_counter: usize,
    num_digits: usize,
    uploader: BoxedLogUploader,
}

impl TempLogStore {
    // 10 MiB per chunk
    const CHUNK_SIZE: usize = 10 * 1024 * 1024;
    // Assume max 1000 log files (10GB of logs), use 3 digits padding
    const NUM_DIGITS: usize = 3;

    pub fn new(uploader: BoxedLogUploader) -> TempLogStore {
        TempLogStore {
            logs: Vec::new(),
            bytes: 0,
            artifact_id: None,
            file_counter: 0,
            num_digits: Self::NUM_DIGITS,
            uploader,
        }
    }

    pub fn push(&mut self, log: String) -> Result<(), LogStoreError> {
        if self.bytes + log.len() > Self::CHUNK_SIZE {
            self.flush()?;
        }
        self.bytes += log.len();
        self.logs.push(log);
        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), LogStoreError> {
        if self.logs.is_empty() {
            return Ok(());
        }

        let full_log = self.logs.join("");
        let log_bytes = full_log.into_bytes();
        let filename = format!(
            "experiment-{:0width$}.log",
            self.file_counter,
            width = self.num_digits
        );

        let bundle = InMemoryBundleSources::new().add_bytes(log_bytes, &filename);

        self.uploader.upload(bundle)?;

        self.logs.clear();
        self.bytes = 0;
        self.file_counter += 1;

        Ok(())
    }
}
