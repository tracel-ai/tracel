use crate::error::BurnCentralClientError;
use crate::http::HttpClient;
use crate::schemas::ExperimentPath;

#[derive(Debug)]
pub struct TempLogStore {
    logs: Vec<String>,
    http_client: HttpClient,
    experiment_path: ExperimentPath,
    bytes: usize,
}

impl TempLogStore {
    // 100 MiB
    const BYTE_LIMIT: usize = 100 * 1024 * 1024;

    pub fn new(http_client: HttpClient, experiment_path: ExperimentPath) -> TempLogStore {
        TempLogStore {
            logs: Vec::new(),
            http_client,
            experiment_path,
            bytes: 0,
        }
    }

    pub fn push(&mut self, log: String) -> Result<(), BurnCentralClientError> {
        if self.bytes + log.len() > Self::BYTE_LIMIT {
            self.flush()?;
        }

        self.bytes += log.len();
        self.logs.push(log);

        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), BurnCentralClientError> {
        if !self.logs.is_empty() {
            let logs_upload_url = self.http_client.request_logs_upload_url(
                self.experiment_path.owner_name(),
                self.experiment_path.project_name(),
                self.experiment_path.experiment_num(),
            )?;
            self.http_client
                .upload_bytes_to_url(&logs_upload_url, self.logs.join("").into_bytes())?;

            self.logs.clear();
            self.bytes = 0;
        }

        Ok(())
    }
}
