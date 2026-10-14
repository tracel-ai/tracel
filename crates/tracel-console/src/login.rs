//! Signing an app in from a device that cannot host a browser session, renewing its
//! access token afterwards without signing in again, and signing it out.

use std::time::Duration;

use tracel_client::console::RefreshToken;
use tracel_client::console::auth::{
    DeviceAuthClient, DeviceFlowError, DevicePollOutcome, IssuedAppSession,
};

use crate::{ConsoleError, env_from_environment};

/// A pending sign-in, and what to put in front of the user while it is pending.
#[derive(Debug, Clone)]
pub struct DeviceLogin {
    client: DeviceAuthClient,
    device_code: String,
    /// Code the user types on the verification page.
    pub user_code: String,
    /// Page the user opens to approve the sign-in.
    pub verification_uri: String,
    /// [`Self::verification_uri`] with the code already filled in.
    pub verification_uri_complete: String,
    /// How long the user has to approve.
    pub expires_in: Duration,
    /// How long to wait between polls.
    pub interval: Duration,
}

impl DeviceLogin {
    /// Asks the console to start a sign-in.
    pub fn start(client_id: impl Into<String>) -> Result<Self, ConsoleError> {
        let client = DeviceAuthClient::new(env_from_environment()?, client_id);
        let started = client.start().map_err(login_failure)?;

        Ok(Self {
            client,
            device_code: started.device_code.clone(),
            user_code: started.user_code.clone(),
            verification_uri: started.verification_uri.clone(),
            verification_uri_complete: started.verification_uri_complete.clone(),
            expires_in: started.expires_in(),
            interval: started.interval(),
        })
    }

    /// Asks once whether the user has answered, without waiting.
    ///
    /// The caller owns the waiting, so a sign-in stays interruptible.
    pub fn poll(&self) -> Result<DeviceApproval, ConsoleError> {
        match self.client.poll(&self.device_code) {
            Ok(DevicePollOutcome::Pending) => Ok(DeviceApproval::Waiting),
            Ok(DevicePollOutcome::SlowDown) => Ok(DeviceApproval::PollLessOften),
            Ok(DevicePollOutcome::Approved(session)) => Ok(DeviceApproval::Approved(session)),
            Err(error) => Err(login_failure(error)),
        }
    }
}

/// What the console answered when asked whether the user has approved yet.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum DeviceApproval {
    /// The user has not answered.
    Waiting,
    /// Answered too soon; wait longer before asking again.
    PollLessOften,
    /// The user approved, and the console signed the app in: an access token and the
    /// refresh token that renews it.
    Approved(IssuedAppSession),
}

/// Renews an app's access token from the refresh token kept since its sign-in.
///
/// The renewal needs no [`DeviceLogin`], only the token and the same `client_id` the
/// sign-in used. Every renewal rotates the refresh token, so the one that comes back
/// replaces the one spent here. [`ConsoleError::RefreshRejected`] is terminal: only a
/// new [`DeviceLogin`] recovers from it.
pub fn refresh_session(
    client_id: impl Into<String>,
    refresh_token: &RefreshToken,
) -> Result<IssuedAppSession, ConsoleError> {
    DeviceAuthClient::new(env_from_environment()?, client_id)
        .refresh(refresh_token)
        .map_err(login_failure)
}

/// Signs an app out with its refresh token, or one of its access tokens.
///
/// The console answers the same whether or not the token was still live, so signing
/// out twice is not an error.
pub fn sign_out(client_id: impl Into<String>, token: &str) -> Result<(), ConsoleError> {
    DeviceAuthClient::new(env_from_environment()?, client_id)
        .revoke(token)
        .map_err(login_failure)
}

/// Reads a sign-in failure without exposing the protocol it was spoken in.
fn login_failure(error: DeviceFlowError) -> ConsoleError {
    match error {
        DeviceFlowError::AccessDenied => ConsoleError::LoginDenied,
        DeviceFlowError::ExpiredToken => ConsoleError::LoginExpired,
        DeviceFlowError::InvalidGrant => ConsoleError::RefreshRejected,
        DeviceFlowError::Client(error) => ConsoleError::from(error),
        error => ConsoleError::InvalidResponse(error.to_string()),
    }
}
