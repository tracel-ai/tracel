use std::sync::Mutex;
use std::{collections::HashMap, sync::Arc};

use serde::Deserialize;
use tungstenite::client;

use crate::error::HeatSDKError;
use crate::websocket::WebSocketClient;
use crate::ws_messages::WsMessage;

pub enum AccessMode {
    Read,
    Write,
}

#[derive(Debug)]

// enum Credentials {
//     ApiKey(String),
//     Login {
//         username: String,
//         password: String,
//     },
// }

pub struct HeatClientConfig {
    pub endpoint: String,
    pub api_key: String, // not used yet, but will be used for authentication through the Heat backend API
    pub num_retries: u8,
}

impl HeatClientConfig {
    pub fn builder(api_key: impl Into<String>) -> HeatClientConfigBuilder {
        HeatClientConfigBuilder::new(api_key)
    }
}

pub struct HeatClientConfigBuilder {
    config: HeatClientConfig,
}

impl HeatClientConfigBuilder {
    pub(crate) fn new(api_key: impl Into<String>) -> HeatClientConfigBuilder {
        HeatClientConfigBuilder {
            config: HeatClientConfig {
                endpoint: "http://127.0.0.1:9001".into(),
                api_key: api_key.into(),
                num_retries: 3,
            },
        }
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> HeatClientConfigBuilder {
        self.config.endpoint = endpoint.into();
        self
    }

    pub fn with_num_retries(mut self, num_retries: u8) -> HeatClientConfigBuilder {
        self.config.num_retries = num_retries;
        self
    }

    pub fn build(self) -> HeatClientConfig {
        self.config
    }
}

#[derive(Debug)]
pub struct HeatClient {
    config: HeatClientConfig,

    http_client: reqwest::blocking::Client,
    ws_client: WebSocketClient,
}

pub type HeatClientState = Arc<Mutex<HeatClient>>;

impl HeatClient {
    fn new(config: HeatClientConfig) -> HeatClient {
        let http_client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("Client should be created.");

        let ws_client = WebSocketClient::new();

        HeatClient {
            config,
            http_client,
            ws_client,
        }
    }

    fn health_check(&self) -> Result<(), reqwest::Error> {
        let url = format!("{}/health", self.get_endpoint());
        self.http_client.get(url).send()?;

        Ok(())
    }

    fn request_ws(&self) -> Result<String, HeatSDKError> {
        #[derive(Deserialize)]
        struct WSURLResponse {
            url: String,
        }

        let json = serde_json::json!({
            "username": "admin",
            "password": "admin"
        });

        let url = format!("{}/wsid", self.get_endpoint());
        let ws_endpoint = self.http_client.get(url)
            .json(&json)
            .send()?.json::<WSURLResponse>()?.url;
        Ok(ws_endpoint)
    }

    pub fn create(config: HeatClientConfig) -> Result<HeatClientState, HeatSDKError> {
        let client_state = Arc::new(Mutex::new(HeatClient::new(config)));

        let client = client_state.lock()?;
        // Try to connect to the api, if it fails, return an error
        for i in 0..=client.config.num_retries {
            let res = client.health_check();
            match res {
                Ok(_) => break,
                Err(e) => {
                    if i == client.config.num_retries {
                        return Err(HeatSDKError::ServerTimeoutError(e.to_string()));
                    }
                    println!("Failed to connect to the server. Retrying in 5 seconds...");
                }
            }
        }

        let ws_endpoint = client.request_ws()?;
        let mut client = client;
        client
            .ws_client
            .connect(ws_endpoint)?;

        drop(client);

        Ok(client_state)
    }

    pub fn get_endpoint(&self) -> String {
        self.config.endpoint.clone()
    }

    pub fn get_api_key(&self) -> String {
        self.config.api_key.clone()
    }

    pub fn get_num_retries(&self) -> u8 {
        self.config.num_retries
    }

    fn request_checkpoint_url(
        &self,
        path: &str,
        access: AccessMode,
    ) -> Result<String, reqwest::Error> {
        let url = format!("{}/checkpoints", self.get_endpoint());

        let mut body = HashMap::new();
        body.insert("file_path", path.to_string());

        #[derive(Deserialize)]
        struct CheckpointURLResponse {
            url: String,
        }

        let response = match access {
            AccessMode::Read => self.http_client.get(url),
            AccessMode::Write => self.http_client.post(url),
        }
        .json(&body)
        .send()?
        .json::<CheckpointURLResponse>()?;

        Ok(response.url)
    }

    fn upload_checkpoint(&self, url: &str, checkpoint: Vec<u8>) -> Result<(), reqwest::Error> {
        self.http_client.put(url).body(checkpoint).send()?;

        Ok(())
    }

    fn download_checkpoint(&self, url: &str) -> Result<Vec<u8>, reqwest::Error> {
        let response = self.http_client.get(url).send()?.bytes()?;

        Ok(response.to_vec())
    }

    pub fn save_checkpoint_data(
        &self,
        path: &str,
        checkpoint: Vec<u8>,
    ) -> Result<(), HeatSDKError> {
        let url = self.request_checkpoint_url(path, AccessMode::Write)?;
        self.upload_checkpoint(&url, checkpoint)?;

        Ok(())
    }

    pub fn load_checkpoint_data(&self, path: &str) -> Result<Vec<u8>, HeatSDKError> {
        let url = self.request_checkpoint_url(path, AccessMode::Read)?;
        let response = self.download_checkpoint(&url)?;

        Ok(response.to_vec())
    }

    pub fn log_experiment(&mut self, message: String) -> Result<(), HeatSDKError> {
        self.ws_client
            .send(WsMessage::Log(message))
            .map_err(|e| HeatSDKError::WebSocketError(e.to_string()))?;
        Ok(())
    }
}
