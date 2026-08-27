use tracel_client::error::ClientError;

/// Errors produced while authenticating with or calling a Tracel console.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConsoleError {
    /// The console base URL is malformed or cannot be used for HTTP requests.
    #[error("invalid console URL: {0}")]
    InvalidUrl(String),
    /// A request could not reach the console or receive its response.
    #[error("console transport failed: {0}")]
    Transport(String),
    /// The session token is no longer accepted by the console.
    #[error("the console session has expired")]
    SessionExpired,
    /// No such resource. The console answers the same way for resources that exist but are
    /// private, so the two cannot be told apart.
    #[error("the console has no such resource")]
    NotFound,
    /// The console response did not match its documented contract.
    #[error("invalid console response: {0}")]
    InvalidResponse(String),
    /// Authentication credentials were rejected.
    #[error("authentication was rejected by the console")]
    AuthenticationRejected,
    /// The device authorization request was rejected before reaching a polling state.
    #[error("device authorization failed: {0}")]
    DeviceAuthorization(#[from] crate::auth::DeviceAuthorizationError),
    /// The console returned an unsuccessful status without a more specific SDK meaning.
    #[error("console returned HTTP {status}: {message}")]
    Server {
        /// HTTP status code returned by the console.
        status: u16,
        /// Human-readable response detail, when one was available.
        message: String,
    },
}

impl ConsoleError {
    /// Returns whether the error means the caller must obtain a new session.
    pub fn is_session_expired(&self) -> bool {
        matches!(self, Self::SessionExpired)
    }
}

impl From<ClientError> for ConsoleError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::Unauthorized => Self::SessionExpired,
            ClientError::NotFound | ClientError::NotFoundWithCode(_) => Self::NotFound,
            ClientError::ApiError { status, .. } if status == reqwest::StatusCode::UNAUTHORIZED => {
                Self::SessionExpired
            }
            ClientError::ApiError { status, .. }
                if status == reqwest::StatusCode::FORBIDDEN
                    || status == reqwest::StatusCode::NOT_FOUND =>
            {
                Self::NotFound
            }
            ClientError::ApiError { status, body } => Self::Server {
                status: status.as_u16(),
                message: body.to_string(),
            },
            ClientError::Serialization(error) => Self::InvalidResponse(error.to_string()),
            ClientError::BadSessionId => {
                Self::InvalidResponse("login response omitted the session cookie".to_string())
            }
            ClientError::InternalServerError => Self::Server {
                status: reqwest::StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                message: "internal server error".to_string(),
            },
            ClientError::UnknownError(message) => Self::Transport(message),
        }
    }
}
