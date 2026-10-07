use tracel_client::error::ClientError;

/// Errors produced while configuring, authenticating with or calling the console.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConsoleError {
    /// The console base URL is malformed or cannot be used for HTTP requests.
    #[error("invalid console URL: {0}")]
    InvalidUrl(String),
    /// A request could not reach the console or receive its response.
    #[error("console transport failed: {0}")]
    Transport(String),
    /// The credential is no longer accepted by the console: an API key that expired or was
    /// deleted, or an app session that ended or was signed out.
    #[error("the console no longer accepts this credential")]
    SessionExpired,
    /// The stored sign-in could not be read, written or locked.
    #[error("the stored sign-in could not be used: {0}")]
    SessionStore(String),
    /// An environment variable holds a value the SDK does not accept.
    #[error("invalid {variable} value `{value}`: expected {expected}")]
    InvalidSetting {
        /// The environment variable.
        variable: &'static str,
        /// The value it holds.
        value: String,
        /// The values it accepts.
        expected: &'static str,
    },
    /// Neither `TRACEL_API_KEY` nor a `tracel login` sign-in provides a credential.
    #[error("no credentials found: set TRACEL_API_KEY or run `tracel login`")]
    NoCredentials,
    /// Neither `TRACEL_NAMESPACE` nor `tracel.toml` names the project's owner namespace.
    #[error("no namespace found: set TRACEL_NAMESPACE or add namespace to tracel.toml")]
    NoNamespace,
    /// Neither `TRACEL_PROJECT` nor `tracel.toml` names the project.
    #[error("no project found: set TRACEL_PROJECT or add project to tracel.toml")]
    NoProject,
    /// The credential cannot be used for this request: an API key reaches project data only.
    #[error("this credential cannot be used for this request; API keys reach project data only")]
    CredentialNotAllowed,
    /// No such resource. The console answers the same way for resources that exist but are
    /// private, so the two cannot be told apart.
    #[error("the console has no such resource")]
    NotFound,
    /// The user refused the sign-in.
    #[error("the sign-in was denied")]
    LoginDenied,
    /// The user did not answer before the codes expired.
    #[error("the sign-in expired before it was approved")]
    LoginExpired,
    /// The stored refresh token was refused, so the session cannot be renewed.
    #[error("the refresh token was rejected; sign in again")]
    RefreshRejected,
    /// The console response did not match its documented contract.
    #[error("invalid console response: {0}")]
    InvalidResponse(String),
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
            ClientError::Unauthenticated | ClientError::AppSessionEnded => Self::SessionExpired,
            ClientError::SessionStore(reason) => Self::SessionStore(reason),
            ClientError::CredentialNotAllowed => Self::CredentialNotAllowed,
            ClientError::NotFound | ClientError::NotFoundWithCode(_) => Self::NotFound,
            ClientError::ApiError { status, .. } if status == reqwest::StatusCode::UNAUTHORIZED => {
                Self::SessionExpired
            }
            ClientError::ApiError { status, .. } if status_is_not_found(status) => Self::NotFound,
            ClientError::ApiError { status, body } => Self::Server {
                status: status.as_u16(),
                message: body.to_string(),
            },
            ClientError::Serialization(error) => Self::InvalidResponse(error.to_string()),
            ClientError::InternalServerError => Self::Server {
                status: reqwest::StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                message: "internal server error".to_string(),
            },
            ClientError::UnknownError(message) => Self::Transport(message),
            error => Self::Transport(error.to_string()),
        }
    }
}

pub fn client_error_is_not_found(error: &ClientError) -> bool {
    error.is_not_found()
        || matches!(
            error,
            ClientError::ApiError { status, .. } if status_is_not_found(*status)
        )
}

fn status_is_not_found(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::NOT_FOUND
}
