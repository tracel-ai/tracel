use reqwest::StatusCode;
use serde::Deserialize;
use strum::Display;
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Display)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[non_exhaustive]
pub enum ApiErrorCode {
    ProjectAlreadyExists,
    LimitReached,
    // ...
    #[serde(other)]
    Unknown,
}

#[derive(Error, Deserialize, Debug)]
#[error("Api error {status}: Code: {code}, Message: {message}")]
pub struct ApiError {
    #[serde(skip)]
    pub status: StatusCode,
    pub code: ApiErrorCode,
    pub message: String,
}

impl Default for ApiError {
    fn default() -> Self {
        ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: ApiErrorCode::Unknown,
            message: "An unknown error occurred".to_string(),
        }
    }
}

impl ApiError {
    pub fn code(&self) -> ApiErrorCode {
        self.code.clone()
    }

    pub fn is_login_error(&self) -> bool {
        self.status == StatusCode::UNAUTHORIZED
    }

    pub fn is_not_found(&self) -> bool {
        self.status == StatusCode::NOT_FOUND
    }
}
