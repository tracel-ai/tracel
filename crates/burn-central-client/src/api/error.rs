use reqwest::StatusCode;
use serde::Deserialize;
use std::fmt::{Display, Formatter};
use strum::{Display, EnumString};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Display)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[non_exhaustive]
pub enum ApiErrorCode {
    ProjectAlreadyExists,
    // ...
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize, Debug)]
pub struct ApiErrorBody {
    pub code: ApiErrorCode,
    pub message: String,
}

impl Default for ApiErrorBody {
    fn default() -> Self {
        ApiErrorBody {
            code: ApiErrorCode::Unknown,
            message: "An unknown error occurred".to_string(),
        }
    }
}

impl Display for ApiErrorBody {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Code: {}, Message: {}", self.code, self.message)
    }
}

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("Bad session id")]
    BadSessionId,
    #[error("Resource not found")]
    NotFound,
    #[error("Unauthorized access")]
    Unauthorized,
    #[error("Forbidden access")]
    Forbidden,
    #[error("Internal server error")]
    InternalServerError,
    #[error("Api error {status}: {body}")]
    ApiError {
        status: StatusCode,
        body: ApiErrorBody,
    },
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    #[error("Unknown Error: {0}")]
    UnknownError(String),
}

impl ClientError {
    pub fn code(&self) -> Option<ApiErrorCode> {
        match self {
            ClientError::ApiError { body, .. } => Some(body.code.clone()),
            _ => None,
        }
    }

    pub fn is_login_error(&self) -> bool {
        matches!(self, ClientError::Unauthorized | ClientError::Forbidden)
    }
}
