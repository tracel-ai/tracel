//! Session acquisition for console clients.
//!
//! Device polling is deliberately exposed one step at a time so interactive callers retain
//! control of sleeping, progress narration, and cancellation.

use std::time::Duration;

use reqwest::header::SET_COOKIE;
use serde::Deserialize;
use url::Url;

use crate::{ConsoleError, SessionToken, normalize_base_url};

const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const SLOW_DOWN_INCREMENT: Duration = Duration::from_secs(5);

/// Client for session-acquisition endpoints that do not yet have a session cookie.
#[derive(Clone, Debug)]
pub struct AuthClient {
    base_url: Url,
    transport: ReqwestAuthTransport,
}

impl AuthClient {
    /// Creates an authentication client without making a network request.
    pub fn new(url: impl AsRef<str>) -> Result<Self, ConsoleError> {
        Ok(Self {
            base_url: normalize_base_url(url.as_ref())?,
            transport: ReqwestAuthTransport::new()?,
        })
    }

    /// Returns the normalized console API base URL.
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Starts a device authorization and returns the code shown to the user.
    pub fn request_code(&self, client_id: &str) -> Result<DeviceCode, ConsoleError> {
        request_code_with(&self.transport, &self.base_url, client_id)
    }

    /// Performs one device-token poll without sleeping or retrying.
    pub fn poll(&self, client_id: &str, code: &DeviceCode) -> Result<DevicePoll, ConsoleError> {
        poll_with(&self.transport, &self.base_url, client_id, code)
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
        exchange_api_key_with(&self.transport, &self.base_url, api_key)
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

#[derive(Clone, Debug)]
struct ReqwestAuthTransport {
    client: reqwest::blocking::Client,
}

impl ReqwestAuthTransport {
    fn new() -> Result<Self, ConsoleError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| ConsoleError::Transport(error.to_string()))?;
        Ok(Self { client })
    }
}

trait AuthTransport {
    fn post_form(&self, url: Url, form: &[(&str, &str)]) -> Result<HttpResponse, ConsoleError>;
}

impl AuthTransport for ReqwestAuthTransport {
    fn post_form(&self, url: Url, form: &[(&str, &str)]) -> Result<HttpResponse, ConsoleError> {
        let response = self
            .client
            .post(url)
            .header("X-SDK-Version", env!("CARGO_PKG_VERSION"))
            .form(form)
            .send()
            .map_err(|error| ConsoleError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
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
            .map_err(|error| ConsoleError::Transport(error.to_string()))?
            .to_vec();

        Ok(HttpResponse {
            status,
            body,
            set_cookies,
        })
    }
}

#[derive(Clone, Debug)]
struct HttpResponse {
    status: u16,
    body: Vec<u8>,
    set_cookies: Vec<String>,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    expires_in: i64,
    interval: i64,
}

#[derive(Deserialize)]
struct SessionResponse {
    session_token: String,
}

#[derive(Deserialize)]
struct OAuthErrorResponse {
    error: OAuthErrorCode,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OAuthErrorCode {
    AccessDenied,
    AuthorizationPending,
    ExpiredToken,
    InvalidRequest,
    SlowDown,
    UnsupportedGrantType,
}

fn request_code_with(
    transport: &impl AuthTransport,
    base_url: &Url,
    client_id: &str,
) -> Result<DeviceCode, ConsoleError> {
    let response = transport.post_form(
        endpoint(base_url, "auth/device/code")?,
        &[("client_id", client_id)],
    )?;
    if response.status == reqwest::StatusCode::BAD_REQUEST.as_u16() {
        return Err(device_request_error(&response.body)?.into());
    }
    ensure_success(&response)?;
    let response: DeviceCodeResponse = parse_json(&response.body)?;
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

fn poll_with(
    transport: &impl AuthTransport,
    base_url: &Url,
    client_id: &str,
    code: &DeviceCode,
) -> Result<DevicePoll, ConsoleError> {
    let response = transport.post_form(
        endpoint(base_url, "auth/token")?,
        &[
            ("grant_type", DEVICE_CODE_GRANT),
            ("device_code", &code.device_code),
            ("client_id", client_id),
        ],
    )?;

    match response.status {
        status if (200..300).contains(&status) => {
            let response: SessionResponse = parse_json(&response.body)?;
            if response.session_token.is_empty() {
                return Err(ConsoleError::InvalidResponse(
                    "approved device response contained an empty session token".to_string(),
                ));
            }
            Ok(DevicePoll::Approved(SessionToken::new(
                response.session_token,
            )))
        }
        status if status == reqwest::StatusCode::BAD_REQUEST.as_u16() => {
            let response: OAuthErrorResponse = parse_json(&response.body)?;
            match response.error {
                OAuthErrorCode::AuthorizationPending => Ok(DevicePoll::Pending),
                OAuthErrorCode::SlowDown => Ok(DevicePoll::SlowDown),
                OAuthErrorCode::AccessDenied => Ok(DevicePoll::Denied),
                OAuthErrorCode::ExpiredToken => Ok(DevicePoll::Expired),
                OAuthErrorCode::InvalidRequest => {
                    Err(DeviceAuthorizationError::InvalidRequest.into())
                }
                OAuthErrorCode::UnsupportedGrantType => {
                    Err(DeviceAuthorizationError::UnsupportedGrantType.into())
                }
            }
        }
        _ => {
            ensure_success(&response)?;
            unreachable!("successful responses are handled above")
        }
    }
}

fn exchange_api_key_with(
    transport: &impl AuthTransport,
    base_url: &Url,
    api_key: &str,
) -> Result<SessionToken, ConsoleError> {
    let response = transport.post_form(
        endpoint(base_url, "login/api-key")?,
        &[("api_key", api_key)],
    )?;
    if response.status == reqwest::StatusCode::UNAUTHORIZED.as_u16() {
        return Err(ConsoleError::AuthenticationRejected);
    }
    ensure_success(&response)?;

    let token = response
        .set_cookies
        .iter()
        .filter_map(|header| header.split(';').next())
        .filter_map(|cookie| cookie.trim().split_once('='))
        .find_map(|(name, value)| (name == "id" && !value.is_empty()).then_some(value));
    token.map(SessionToken::new).ok_or_else(|| {
        ConsoleError::InvalidResponse("login response omitted the `id` cookie".into())
    })
}

fn endpoint(base_url: &Url, path: &str) -> Result<Url, ConsoleError> {
    base_url
        .join("v1/")
        .and_then(|base| base.join(path))
        .map_err(|error| ConsoleError::InvalidUrl(error.to_string()))
}

fn device_request_error(body: &[u8]) -> Result<DeviceAuthorizationError, ConsoleError> {
    let response: OAuthErrorResponse = parse_json(body)?;
    match response.error {
        OAuthErrorCode::InvalidRequest => Ok(DeviceAuthorizationError::InvalidRequest),
        OAuthErrorCode::UnsupportedGrantType => Ok(DeviceAuthorizationError::UnsupportedGrantType),
        error => Err(ConsoleError::InvalidResponse(format!(
            "device code endpoint returned unexpected error `{error:?}`"
        ))),
    }
}

fn parse_json<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, ConsoleError> {
    serde_json::from_slice(body).map_err(|error| ConsoleError::InvalidResponse(error.to_string()))
}

fn ensure_success(response: &HttpResponse) -> Result<(), ConsoleError> {
    if (200..300).contains(&response.status) {
        return Ok(());
    }
    if response.status == reqwest::StatusCode::UNAUTHORIZED.as_u16() {
        return Err(ConsoleError::SessionExpired);
    }
    if response.status == reqwest::StatusCode::FORBIDDEN.as_u16()
        || response.status == reqwest::StatusCode::NOT_FOUND.as_u16()
    {
        return Err(ConsoleError::NotVisible);
    }

    let message = serde_json::from_slice::<serde_json::Value>(&response.body)
        .ok()
        .and_then(|body| {
            body.get("message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(&response.body).trim().to_string());
    Err(ConsoleError::Server {
        status: response.status,
        message,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    struct FakeTransport {
        responses: Mutex<VecDeque<HttpResponse>>,
    }

    impl FakeTransport {
        fn new(responses: impl IntoIterator<Item = HttpResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().collect()),
            }
        }
    }

    impl AuthTransport for FakeTransport {
        fn post_form(
            &self,
            _url: Url,
            _form: &[(&str, &str)],
        ) -> Result<HttpResponse, ConsoleError> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| ConsoleError::Transport("fake response queue is empty".to_string()))
        }
    }

    fn response(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            body: body.as_bytes().to_vec(),
            set_cookies: Vec::new(),
        }
    }

    fn code() -> DeviceCode {
        DeviceCode {
            device_code: "device-secret".to_string(),
            user_code: "ABCD-EFGH".to_string(),
            verification_uri: "https://console.example/activate".to_string(),
            verification_uri_complete: "https://console.example/activate?code=ABCD-EFGH"
                .to_string(),
            expires_in: Duration::from_secs(900),
            interval: Duration::from_secs(5),
        }
    }

    #[test]
    fn request_code_deserializes_the_server_fixture() {
        let transport = FakeTransport::new([response(
            200,
            r#"{
                "device_code":"device-secret",
                "user_code":"ABCD-EFGH",
                "verification_uri":"https://console.example/activate",
                "verification_uri_complete":"https://console.example/activate?code=ABCD-EFGH",
                "expires_in":900,
                "interval":5
            }"#,
        )]);
        let base_url = Url::parse("https://console.example/api/").unwrap();

        let code = request_code_with(&transport, &base_url, "metabolic").unwrap();

        assert_eq!(code.user_code, "ABCD-EFGH");
        assert_eq!(code.expires_in, Duration::from_secs(900));
        assert_eq!(code.interval, Duration::from_secs(5));
        assert!(!format!("{code:?}").contains("device-secret"));
    }

    #[test]
    fn poll_maps_each_protocol_state_over_a_fake_transport() {
        let transport = FakeTransport::new([
            response(400, r#"{"error":"authorization_pending"}"#),
            response(400, r#"{"error":"slow_down"}"#),
            response(200, r#"{"session_token":"session-1"}"#),
            response(400, r#"{"error":"access_denied"}"#),
            response(400, r#"{"error":"expired_token"}"#),
        ]);
        let base_url = Url::parse("https://console.example/api/").unwrap();

        assert_eq!(
            poll_with(&transport, &base_url, "metabolic", &code()).unwrap(),
            DevicePoll::Pending
        );
        assert_eq!(
            poll_with(&transport, &base_url, "metabolic", &code()).unwrap(),
            DevicePoll::SlowDown
        );
        assert_eq!(
            poll_with(&transport, &base_url, "metabolic", &code()).unwrap(),
            DevicePoll::Approved(SessionToken::new("session-1"))
        );
        assert_eq!(
            poll_with(&transport, &base_url, "metabolic", &code()).unwrap(),
            DevicePoll::Denied
        );
        assert_eq!(
            poll_with(&transport, &base_url, "metabolic", &code()).unwrap(),
            DevicePoll::Expired
        );
    }

    #[test]
    fn api_key_exchange_extracts_only_the_session_cookie_value() {
        let mut response = response(200, "");
        response
            .set_cookies
            .push("id=session-2; HttpOnly; SameSite=Lax; Path=/".to_string());
        let transport = FakeTransport::new([response]);
        let base_url = Url::parse("https://console.example/api/").unwrap();

        let token = exchange_api_key_with(&transport, &base_url, "api-key").unwrap();

        assert_eq!(token.expose_secret(), "session-2");
    }
}
