use tracel_client::error::ClientError;

/// Errors produced while authenticating with or calling the console.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_credential_the_console_no_longer_accepts_reads_as_an_expired_session() {
        assert!(ConsoleError::from(ClientError::Unauthenticated).is_session_expired());
    }

    #[test]
    fn a_credential_a_route_does_not_accept_is_not_mistaken_for_a_missing_resource() {
        assert!(matches!(
            ConsoleError::from(ClientError::CredentialNotAllowed),
            ConsoleError::CredentialNotAllowed
        ));
    }

    #[test]
    fn an_app_session_that_ended_reads_as_an_expired_session() {
        assert!(ConsoleError::from(ClientError::AppSessionEnded).is_session_expired());
    }
}
