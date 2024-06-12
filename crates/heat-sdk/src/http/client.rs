use std::collections::HashMap;

use reqwest::header::{COOKIE, SET_COOKIE};
use serde::Serialize;

use crate::{client::HeatCredentials, error::HeatSdkError, http::schemas::StartExperimentSchema};

use super::schemas::{CreateExperimentResponseSchema, EndExperimentSchema, URLSchema};


pub enum EndExperimentStatus {
    Success,
    Fail(String),
}

/// A client for making HTTP requests to the Heat API.
#[derive(Debug, Clone)]
pub struct HttpClient {
    http_client: reqwest::blocking::Client,
    base_url: String,
    // credentials: HeatCredentials,
    session_cookie: Option<String>,
}

impl HttpClient {
    /// Create a new HttpClient with the given base URL and API key.
    pub fn new(base_url: String) -> Self {
        Self {
            http_client: reqwest::blocking::Client::new(),
            base_url,
            // credentials,
            session_cookie: None,
        }
    }

    /// Create a new HttpClient with the given base URL, API key, and session cookie.
    pub fn with_session_cookie(base_url: String, session_cookie: String) -> Self {
        Self {
            http_client: reqwest::blocking::Client::new(),
            base_url,
            // credentials,
            session_cookie: Some(session_cookie),
        }
    }

    pub fn health_check(&self) -> Result<(), HeatSdkError> {
        let url = format!("{}/health", self.base_url);
        self.http_client.get(url).send()?.error_for_status()?;
        Ok(())
    }

    pub fn login(&mut self, credentials: &HeatCredentials) -> Result<(), HeatSdkError> {
        let url = format!("{}/login/api-key", self.base_url);
        let res = self
            .http_client
            .post(url)
            .form::<HeatCredentials>(credentials)
            .send()?;

        // store session cookie
        if res.status().is_success() {
            let cookie_header = res.headers().get(SET_COOKIE);
            if let Some(cookie) = cookie_header {
                let cookie_str = cookie
                    .to_str()
                    .expect("Session cookie should be convert to str");
                self.session_cookie = Some(cookie_str.to_string());
            } else {
                return Err(HeatSdkError::ClientError(
                    "Cannot connect to Heat server, bad session ID.".to_string(),
                ));
            }
        } else {
            let error_message: String = format!("Cannot connect to Heat server({:?})", res.text()?);
            return Err(HeatSdkError::ClientError(error_message));
        }
    
        Ok(())
    }

    pub fn request_ws(&self, exp_uuid: &str) -> Result<String, HeatSdkError> {
        self.validate_session_cookie()?;

        let url = format!(
            "{}/experiments/{}/ws",
            self.base_url.clone(),
            exp_uuid
        );
        let ws_endpoint = self
            .http_client
            .get(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .error_for_status()?
            .json::<URLSchema>()?
            .url;
        Ok(ws_endpoint)
    }

    pub fn create_experiment(&self, project_id: &str) -> Result<String, HeatSdkError> {
        self.validate_session_cookie()?;

        let url = format!(
            "{}/projects/{}/experiments",
            self.base_url,
            project_id
        );

        // Create a new experiment
        let exp_uuid = self
            .http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .error_for_status()?
            .json::<CreateExperimentResponseSchema>()?
            .experiment_id;

        Ok(exp_uuid)
    }

    pub fn start_experiment(&self, exp_uuid: &str, config: &impl Serialize) -> Result<(), HeatSdkError> {
        self.validate_session_cookie()?;

        let json = StartExperimentSchema {
            config: serde_json::to_value(config).unwrap(),
        };

        // Start the experiment
        self.http_client
            .put(format!(
                "{}/experiments/{}/start",
                self.base_url,
                exp_uuid
            ))
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&json)
            .send()?
            .error_for_status()?;

        Ok(())
    }

    pub fn end_experiment(&self, exp_uuid: &str, end_status: EndExperimentStatus) -> Result<(), HeatSdkError> {
        self.validate_session_cookie()?;

        let url = format!("{}/experiments/{}/end", self.base_url, exp_uuid);

        let end_status: EndExperimentSchema = match end_status {
            EndExperimentStatus::Success => EndExperimentSchema::Success,
            EndExperimentStatus::Fail(reason) => EndExperimentSchema::Fail(reason),
        };

        self.http_client
            .put(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&end_status)
            .send()?
            .error_for_status()?;

        Ok(())
    }

    pub fn request_checkpoint_save_url(&self, exp_uuid: &str, path: &str) -> Result<String, HeatSdkError> {
        self.validate_session_cookie()?;

        let url: String = format!("{}/checkpoints", self.base_url);

        let mut body = HashMap::new();
        body.insert("file_path", path.to_string());
        body.insert("experiment_id", exp_uuid.to_string());

        let save_url = self.http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&body)
            .send()?
            .error_for_status()?
            .json::<URLSchema>()
            .map(|res| res.url)?;

        Ok(save_url)
    }

    pub fn request_checkpoint_load_url(&self, exp_uuid: &str, path: &str) -> Result<String, HeatSdkError> {
        self.validate_session_cookie()?;

        let url: String = format!("{}/checkpoints/load", self.base_url);

        let mut body = HashMap::new();
        body.insert("file_path", path.to_string());
        body.insert("experiment_id", exp_uuid.to_string());

        let load_url = self.http_client
            .get(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&body)
            .send()?
            .error_for_status()?
            .json::<URLSchema>()
            .map(|res| res.url)?;

        Ok(load_url)
    }

    pub fn request_final_model_save_url(&self, exp_uuid: &str) -> Result<String, HeatSdkError> {
        self.validate_session_cookie()?;

        let url = format!("{}/experiments/{}/save_model", self.base_url, exp_uuid);

        let save_url = self.http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .error_for_status()?
            .json::<URLSchema>()?
            .url;

        Ok(save_url)
    }

    pub fn request_logs_upload_url(&self, exp_uuid: &str) -> Result<String, HeatSdkError> {
        self.validate_session_cookie()?;

        let url = format!("{}/experiments/{}/logs", self.base_url, exp_uuid);

        let logs_upload_url = self.http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .error_for_status()?
            .json::<URLSchema>()?
            .url;

        Ok(logs_upload_url)
    }

    pub fn upload_bytes_to_url(&self, url: &str, bytes: Vec<u8>) -> Result<(), HeatSdkError> {
        self.http_client
            .put(url)
            .body(bytes)
            .send()?
            .error_for_status()?;

        Ok(())
    }

    pub fn download_bytes_from_url(&self, url: &str) -> Result<Vec<u8>, HeatSdkError> {
        let data = self.http_client
            .get(url)
            .send()?
            .error_for_status()?
            .bytes()?
            .to_vec();

        Ok(data)
    }

    fn validate_session_cookie(&self) -> Result<(), HeatSdkError> {
        if self.session_cookie.is_none() {
            return Err(HeatSdkError::ClientError(
                "Cannot connect to Heat server, no session ID.".to_string(),
            ));
        }
        Ok(())
    }

}
