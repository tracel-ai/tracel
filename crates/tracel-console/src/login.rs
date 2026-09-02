//! Signing in from a device that cannot host a browser session.

use std::time::Duration;

use tracel_client::console::SessionToken;
use tracel_client::console::auth::{DeviceAuthClient, DeviceFlowError, DevicePollOutcome};

use crate::ConsoleError;

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
        let client = DeviceAuthClient::new(crate::env::from_environment(), client_id);
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
            Ok(DevicePollOutcome::Approved(token)) => Ok(DeviceApproval::Approved(token)),
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
    /// The user approved, and the console issued this session.
    Approved(SessionToken),
}

/// Reads a sign-in failure without exposing the protocol it was spoken in.
fn login_failure(error: DeviceFlowError) -> ConsoleError {
    match error {
        DeviceFlowError::AccessDenied => ConsoleError::LoginDenied,
        DeviceFlowError::ExpiredToken => ConsoleError::LoginExpired,
        DeviceFlowError::Client(error) => ConsoleError::from(error),
        error => ConsoleError::InvalidResponse(error.to_string()),
    }
}
