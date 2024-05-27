use crate::{error::HeatSdkError, http_schemas::URLSchema, websocket::WebSocketClient, ws_messages::WsMessage};

#[derive(Debug)]
pub struct TempLogStore {
    logs: Vec<String>,
    http_client: reqwest::blocking::Client,
    endpoint: String,
    exp_id: String,
}

impl TempLogStore {
    const LOG_LIMIT: usize = 1000;

    pub fn new(http_client: reqwest::blocking::Client, endpoint: String, exp_id: String) -> TempLogStore {
        TempLogStore {
            logs: Vec::new(),
            http_client: http_client,
            endpoint: endpoint,
            exp_id: exp_id,
        }
    }

    pub fn push(&mut self, log: String) -> Result<(), HeatSdkError> {
        if self.logs.len() >= Self::LOG_LIMIT {

            let logs_upload_url = self
                .http_client
                .post(format!(
                    "{}/experiments/{}/logs",
                    self.endpoint,
                    self.exp_id
                ))
                .send()?
                .json::<URLSchema>()?
                .url;

            self.http_client.put(logs_upload_url).body(self.logs.join("")).send()?;

            self.logs.clear();
        }
        self.logs.push(log);

        Ok(())
    }

    pub fn logs(&self) -> &Vec<String> {
        &self.logs
    }
    
}

#[derive(Debug)]
pub struct Experiment {
    id: String,
    in_memory_logs: TempLogStore,
    ws_client: WebSocketClient,
}

impl Experiment {
    pub fn new(id: String, ws_client: WebSocketClient, log_store: TempLogStore) -> Experiment {
        assert!(ws_client.state().is_open());
        Experiment {
            id,
            in_memory_logs: log_store,
            ws_client: ws_client,
        }
    }

    pub fn add_log(&mut self, log: String) -> Result<(), HeatSdkError> {
        self.in_memory_logs.push(log.clone())?;
        self.ws_client.send(WsMessage::Log(log))?;

        Ok(())
    }

    pub fn id(&self) -> &String {
        &self.id
    }

    pub fn logs(&self) -> &Vec<String> {
        self.in_memory_logs.logs()
    }
}

impl Drop for Experiment {
    fn drop(&mut self) {
        self.ws_client.close().unwrap();
    }
}
