//! Session acquisition for console clients.
//!
//! Device polling is deliberately exposed one step at a time so interactive callers retain
//! control of sleeping, progress narration, and cancellation.

use std::time::Duration;

use reqwest::header::SET_COOKIE;
use serde::Deserialize;
use tracel_client::console::{
    SessionToken as ClientSessionToken,
    auth::{
        DeviceAuthClient, DeviceCodeResponse, DeviceFlowError, DevicePollOutcome, OAuthErrorCode,
    },
};
use tracel_client::error::ClientError;
use url::Url;

use crate::{ConsoleError, SessionToken, normalize_base_url};

const SLOW_DOWN_INCREMENT: Duration = Duration::from_secs(5);

/// Client for session-acquisition endpoints that do not yet have a session cookie.
#[derive(Clone, Debug)]
pub struct AuthClient {
    base_url: Url,
    api_key_client: reqwest::blocking::Client,
}

impl AuthClient {
    /// Creates an authentication client without making a network request.
    pub fn new(url: impl AsRef<str>) -> Result<Self, ConsoleError> {
        let base_url = normalize_base_url(url.as_ref())?;
        let api_key_client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| ConsoleError::Transport(error.to_string()))?;
        Ok(Self {
            base_url,
            api_key_client,
        })
    }

    /// Returns the normalized console API base URL.
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Starts a device authorization and returns the code shown to the user.
    pub fn request_code(&self, client_id: &str) -> Result<DeviceCode, ConsoleError> {
        let response = self
            .device_client(client_id)
            .start()
            .map_err(map_device_start_error)?;
        device_code_from_client(response)
    }

    /// Performs one device-token poll without sleeping or retrying.
    pub fn poll(&self, client_id: &str, code: &DeviceCode) -> Result<DevicePoll, ConsoleError> {
        match self.device_client(client_id).poll(&code.device_code) {
            Ok(outcome) => device_poll_from_client(outcome),
            Err(error) => device_poll_from_error(error),
        }
    }

    /// Polls until the console approves, denies, or expires the device authorization.
    ///
    /// The first poll waits for the server-provided interval. A `slow_down` response adds five
    /// seconds to subsequent waits, as required by the device authorization protocol. Interactive
    /// applications should normally use [`Self::poll`] so their own loop can remain cancellable.
    pub fn poll_until_resolved(
        &self,
        client_id: &str,
        code: &DeviceCode,
    ) -> Result<DevicePoll, ConsoleError> {
        let mut interval = code.interval;
        loop {
            std::thread::sleep(interval);
            match self.poll(client_id, code)? {
                DevicePoll::Pending => {}
                DevicePoll::SlowDown => interval = interval.saturating_add(SLOW_DOWN_INCREMENT),
                terminal => return Ok(terminal),
            }
        }
    }

    /// Polls until approval and returns the issued session token.
    ///
    /// Denial and expiration are returned as typed [`DeviceAuthorizationError`] values. See
    /// [`Self::poll_until_resolved`] for timing behavior.
    pub fn poll_until_approved(
        &self,
        client_id: &str,
        code: &DeviceCode,
    ) -> Result<SessionToken, ConsoleError> {
        match self.poll_until_resolved(client_id, code)? {
            DevicePoll::Approved(token) => Ok(token),
            DevicePoll::Denied => Err(DeviceAuthorizationError::Denied.into()),
            DevicePoll::Expired => Err(DeviceAuthorizationError::Expired.into()),
            DevicePoll::Pending | DevicePoll::SlowDown => {
                unreachable!("the blocking poll loop returns only terminal states")
            }
        }
    }

    /// Exchanges an API key for the session token carried by the console's `id` cookie.
    pub fn exchange_api_key(&self, api_key: &str) -> Result<SessionToken, ConsoleError> {
        let response = self
            .api_key_client
            .post(endpoint(&self.base_url, "login/api-key")?)
            .header("X-SDK-Version", env!("CARGO_PKG_VERSION"))
            .form(&[("api_key", api_key)])
            .send()
            .map_err(|error| ConsoleError::Transport(error.to_string()))?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ConsoleError::AuthenticationRejected);
        }

        let set_cookies = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|value| {
                value
                    .to_str()
                    .map(str::to_owned)
                    .map_err(|error| ConsoleError::InvalidResponse(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let body = response
            .bytes()
            .map_err(|error| ConsoleError::Transport(error.to_string()))?;
        ensure_success(status, &body)?;

        session_token_from_set_cookies(&set_cookies).ok_or_else(|| {
            ConsoleError::InvalidResponse("login response omitted the `id` cookie".into())
        })
    }

    fn device_client(&self, client_id: &str) -> DeviceAuthClient {
        DeviceAuthClient::from_url(self.base_url.clone(), client_id)
    }
}

/// Starts a device authorization against `url` using a short-lived authentication client.
pub fn request_code(url: impl AsRef<str>, client_id: &str) -> Result<DeviceCode, ConsoleError> {
    AuthClient::new(url)?.request_code(client_id)
}

/// Performs one device-token poll against `url` without sleeping or retrying.
pub fn poll(
    url: impl AsRef<str>,
    client_id: &str,
    code: &DeviceCode,
) -> Result<DevicePoll, ConsoleError> {
    AuthClient::new(url)?.poll(client_id, code)
}

/// Blocks until device approval and returns the issued session token.
pub fn poll_until_approved(
    url: impl AsRef<str>,
    client_id: &str,
    code: &DeviceCode,
) -> Result<SessionToken, ConsoleError> {
    AuthClient::new(url)?.poll_until_approved(client_id, code)
}

/// Exchanges an API key for a console session token.
pub fn exchange_api_key(url: impl AsRef<str>, api_key: &str) -> Result<SessionToken, ConsoleError> {
    AuthClient::new(url)?.exchange_api_key(api_key)
}

/// A device authorization issued by the console.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceCode {
    device_code: String,
    /// Short code the user enters on the verification page.
    pub user_code: String,
    /// Verification page without a prefilled code.
    pub verification_uri: String,
    /// Verification page with the user code already included.
    pub verification_uri_complete: String,
    /// Lifetime of the device authorization.
    pub expires_in: Duration,
    /// Minimum delay between token polls.
    pub interval: Duration,
}

impl std::fmt::Debug for DeviceCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceCode")
            .field("device_code", &"[REDACTED]")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .field("verification_uri_complete", &self.verification_uri_complete)
            .field("expires_in", &self.expires_in)
            .field("interval", &self.interval)
            .finish()
    }
}

/// Result of one device-token poll.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DevicePoll {
    /// The user has not made a decision yet.
    Pending,
    /// The server requires subsequent polls to wait five additional seconds.
    SlowDown,
    /// The user approved the request and the console issued a session.
    Approved(SessionToken),
    /// The user denied the request.
    Denied,
    /// The device authorization expired before approval.
    Expired,
}

/// RFC 6749-style errors that indicate a malformed device authorization request.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum DeviceAuthorizationError {
    /// The form was missing a required value or contained an invalid one.
    #[error("invalid request")]
    InvalidRequest,
    /// The requested OAuth grant type is not supported.
    #[error("unsupported grant type")]
    UnsupportedGrantType,
    /// The user denied the device authorization.
    #[error("access denied")]
    Denied,
    /// The device authorization expired before approval.
    #[error("device code expired")]
    Expired,
}

fn device_code_from_client(response: DeviceCodeResponse) -> Result<DeviceCode, ConsoleError> {
    let expires_in = u64::try_from(response.expires_in).map_err(|_| {
        ConsoleError::InvalidResponse("device expiration cannot be negative".to_string())
    })?;
    let interval = u64::try_from(response.interval).map_err(|_| {
        ConsoleError::InvalidResponse("device polling interval cannot be negative".to_string())
    })?;

    Ok(DeviceCode {
        device_code: response.device_code,
        user_code: response.user_code,
        verification_uri: response.verification_uri,
        verification_uri_complete: response.verification_uri_complete,
        expires_in: Duration::from_secs(expires_in),
        interval: Duration::from_secs(interval),
    })
}

fn device_poll_from_client(outcome: DevicePollOutcome) -> Result<DevicePoll, ConsoleError> {
    match outcome {
        DevicePollOutcome::Pending => Ok(DevicePoll::Pending),
        DevicePollOutcome::SlowDown => Ok(DevicePoll::SlowDown),
        DevicePollOutcome::Approved(token) => {
            session_token_from_client(token).map(DevicePoll::Approved)
        }
    }
}

fn device_poll_from_error(error: DeviceFlowError) -> Result<DevicePoll, ConsoleError> {
    match error {
        DeviceFlowError::AccessDenied => Ok(DevicePoll::Denied),
        DeviceFlowError::ExpiredToken | DeviceFlowError::TimedOut(_) => Ok(DevicePoll::Expired),
        DeviceFlowError::OAuth(OAuthErrorCode::AccessDenied) => Ok(DevicePoll::Denied),
        DeviceFlowError::OAuth(OAuthErrorCode::ExpiredToken) => Ok(DevicePoll::Expired),
        DeviceFlowError::OAuth(OAuthErrorCode::AuthorizationPending) => Ok(DevicePoll::Pending),
        DeviceFlowError::OAuth(OAuthErrorCode::SlowDown) => Ok(DevicePoll::SlowDown),
        error => Err(map_device_flow_error(error)),
    }
}

fn session_token_from_client(token: ClientSessionToken) -> Result<SessionToken, ConsoleError> {
    let token = token.into_string();
    if token.is_empty() {
        return Err(ConsoleError::InvalidResponse(
            "approved device response contained an empty session token".to_string(),
        ));
    }
    Ok(SessionToken::new(token))
}

fn map_device_start_error(error: DeviceFlowError) -> ConsoleError {
    match error {
        DeviceFlowError::OAuth(OAuthErrorCode::InvalidRequest) => {
            DeviceAuthorizationError::InvalidRequest.into()
        }
        DeviceFlowError::OAuth(OAuthErrorCode::UnsupportedGrantType) => {
            DeviceAuthorizationError::UnsupportedGrantType.into()
        }
        DeviceFlowError::Client(error) => map_device_client_error(error),
        error => ConsoleError::InvalidResponse(format!(
            "device code endpoint returned unexpected error `{error}`"
        )),
    }
}

fn map_device_flow_error(error: DeviceFlowError) -> ConsoleError {
    match error {
        DeviceFlowError::AccessDenied => DeviceAuthorizationError::Denied.into(),
        DeviceFlowError::ExpiredToken | DeviceFlowError::TimedOut(_) => {
            DeviceAuthorizationError::Expired.into()
        }
        DeviceFlowError::OAuth(OAuthErrorCode::InvalidRequest) => {
            DeviceAuthorizationError::InvalidRequest.into()
        }
        DeviceFlowError::OAuth(OAuthErrorCode::UnsupportedGrantType) => {
            DeviceAuthorizationError::UnsupportedGrantType.into()
        }
        DeviceFlowError::OAuth(code) => ConsoleError::InvalidResponse(format!(
            "device authorization returned unexpected OAuth error `{code}`"
        )),
        DeviceFlowError::Client(error) => map_device_client_error(error),
        error => ConsoleError::InvalidResponse(error.to_string()),
    }
}

fn map_device_client_error(error: ClientError) -> ConsoleError {
    match error {
        ClientError::ApiError { status, body } if status == reqwest::StatusCode::BAD_REQUEST => {
            ConsoleError::InvalidResponse(format!(
                "device authorization returned an invalid error response: {body}"
            ))
        }
        error => ConsoleError::from(error),
    }
}

fn session_token_from_set_cookies(headers: &[String]) -> Option<SessionToken> {
    headers
        .iter()
        .filter_map(|header| header.split(';').next())
        .filter_map(|cookie| cookie.trim().split_once('='))
        .find_map(|(name, value)| (name == "id" && !value.is_empty()).then_some(value))
        .map(SessionToken::new)
}

fn endpoint(base_url: &Url, path: &str) -> Result<Url, ConsoleError> {
    base_url
        .join("v1/")
        .and_then(|base| base.join(path))
        .map_err(|error| ConsoleError::InvalidUrl(error.to_string()))
}

fn ensure_success(status: reqwest::StatusCode, body: &[u8]) -> Result<(), ConsoleError> {
    if status.is_success() {
        return Ok(());
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(ConsoleError::SessionExpired);
    }
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::NOT_FOUND {
        return Err(ConsoleError::NotVisible);
    }

    let message = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|body| {
            body.get("message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_string());
    Err(ConsoleError::Server {
        status: status.as_u16(),
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_code(expires_in: i64, interval: i64) -> DeviceCodeResponse {
        DeviceCodeResponse {
            device_code: "device-secret".to_string(),
            user_code: "ABCD-EFGH".to_string(),
            verification_uri: "https://console.example/activate".to_string(),
            verification_uri_complete: "https://console.example/activate?code=ABCD-EFGH"
                .to_string(),
            expires_in,
            interval,
        }
    }

    #[test]
    fn request_code_preserves_public_fields_and_redacts_the_device_secret() {
        let code = device_code_from_client(client_code(900, 5)).unwrap();

        assert_eq!(code.user_code, "ABCD-EFGH");
        assert_eq!(code.expires_in, Duration::from_secs(900));
        assert_eq!(code.interval, Duration::from_secs(5));
        assert!(!format!("{code:?}").contains("device-secret"));
    }

    #[test]
    fn request_code_rejects_negative_server_durations() {
        assert!(matches!(
            device_code_from_client(client_code(-1, 5)),
            Err(ConsoleError::InvalidResponse(_))
        ));
        assert!(matches!(
            device_code_from_client(client_code(900, -1)),
            Err(ConsoleError::InvalidResponse(_))
        ));
    }

    #[test]
    fn poll_maps_client_outcomes_and_terminal_errors_to_the_public_surface() {
        assert_eq!(
            device_poll_from_client(DevicePollOutcome::Pending).unwrap(),
            DevicePoll::Pending
        );
        assert_eq!(
            device_poll_from_client(DevicePollOutcome::SlowDown).unwrap(),
            DevicePoll::SlowDown
        );
        assert_eq!(
            device_poll_from_client(DevicePollOutcome::Approved(ClientSessionToken::new(
                "session-1"
            )))
            .unwrap(),
            DevicePoll::Approved(SessionToken::new("session-1"))
        );
        assert_eq!(
            device_poll_from_error(DeviceFlowError::AccessDenied).unwrap(),
            DevicePoll::Denied
        );
        assert_eq!(
            device_poll_from_error(DeviceFlowError::ExpiredToken).unwrap(),
            DevicePoll::Expired
        );
    }

    #[test]
    fn malformed_device_requests_keep_the_typed_error_mapping() {
        assert!(matches!(
            map_device_start_error(DeviceFlowError::OAuth(OAuthErrorCode::InvalidRequest)),
            ConsoleError::DeviceAuthorization(DeviceAuthorizationError::InvalidRequest)
        ));
        assert!(matches!(
            map_device_start_error(DeviceFlowError::OAuth(OAuthErrorCode::UnsupportedGrantType)),
            ConsoleError::DeviceAuthorization(DeviceAuthorizationError::UnsupportedGrantType)
        ));
    }

    #[test]
    fn unexpected_start_errors_keep_the_legacy_invalid_response_variant() {
        assert!(matches!(
            map_device_start_error(DeviceFlowError::AccessDenied),
            ConsoleError::InvalidResponse(_)
        ));

        let error = ClientError::ApiError {
            status: reqwest::StatusCode::BAD_REQUEST,
            body: tracel_client::error::ApiErrorBody {
                code: tracel_client::ApiErrorCode::Unknown,
                message: "invalid client id".to_string(),
            },
        };
        assert!(matches!(
            map_device_start_error(DeviceFlowError::Client(error)),
            ConsoleError::InvalidResponse(_)
        ));
    }

    #[test]
    fn api_key_exchange_extracts_only_the_session_cookie_value() {
        let token = session_token_from_set_cookies(&[
            "other=ignored; Path=/".to_string(),
            "id=session-2; HttpOnly; SameSite=Lax; Path=/".to_string(),
        ])
        .unwrap();

        assert_eq!(token.expose_secret(), "session-2");
    }
}
