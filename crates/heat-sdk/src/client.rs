use std::{collections::HashMap, sync::Arc};

use serde::Deserialize;

#[derive(Debug)]
pub struct HeatSDKConfig {
    endpoint: String,
    api_key: String,
}

impl HeatSDKConfig {
    pub fn new(endpoint: String, api_key: String) -> HeatSDKConfig {
        HeatSDKConfig {
            endpoint,
            api_key,
        }
    }

    pub fn get_endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn get_api_key(&self) -> &str {
        &self.api_key
    }
}

#[derive(Debug)]
pub struct HeatClient
{
    config: HeatSDKConfig,

    http_client: reqwest::blocking::Client,
}

type HeatClientState = Arc<HeatClient>;

impl HeatClient
{
    fn new(config: HeatSDKConfig) -> HeatClient
    {
        HeatClient {
            config,
            http_client: reqwest::blocking::Client::new(),
        }
    }

    pub fn create(config: HeatSDKConfig) -> HeatClientState
    {
        Arc::new(HeatClient::new(config))
    }

    pub fn get_endpoint(&self) -> &str
    {
        self.config.get_endpoint()
    }

    pub fn get_api_key(&self) -> &str
    {
        self.config.get_api_key()
    }

    fn request_checkpoint_url(&self, path: &str) -> Result<String, reqwest::Error>
    {
        let url = format!("{}/checkpoints", self.get_endpoint());

        let mut body = HashMap::new();
        body.insert("file_path", path.to_string());


        #[derive(Deserialize)]
        struct CheckpointURLResponse {
            url: String,
        }
        
        let response = self.http_client
            .post(url)
            // .header("Authorization", format!("Bearer {}", self.get_api_key()))
            .json(&body)
            .send()?
            .json::<CheckpointURLResponse>()?;

        Ok(response.url)
    }

    fn upload_checkpoint(&self, url: &str, checkpoint: Vec<u8>) -> Result<(), reqwest::Error>
    {

        self.http_client
            .put(url)
            .body(checkpoint)
            .send()?;

        Ok(())
    }

    pub fn save_checkpoint_data(&self, path: &str, checkpoint: Vec<u8>) -> Result<(), reqwest::Error>
    {
        let url = self.request_checkpoint_url(path)?;
        self.upload_checkpoint(&url, checkpoint)
    }

}