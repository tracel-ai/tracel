//! Wire types for the station's runner protocol.
//!
//! Mirrors the station's `/v1/runners/events` (registration + SSE signals) and
//! `/v1/jobs/{id}/finish` contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::job::JobDefinition;

/// Registration body; the response to it is the SSE event stream.
#[derive(Debug, Clone, Serialize)]
pub struct RegisterRunner {
    /// Optional display label. Duplicates are allowed — identity is the session.
    pub name: Option<String>,
    /// The jobs this runner can execute.
    pub jobs: Vec<JobDefinition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishStatus {
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinishJob {
    /// The session id received in the `registered` event.
    pub runner_id: Uuid,
    pub status: FinishStatus,
    /// Failure detail when status is `failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The full job payload the station pushes — dispatch is explicit, there is no claim round-trip.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DispatchedJob {
    pub id: Uuid,
    pub job_name: String,
    pub input: Value,
}

/// An event received on the runner's stream.
#[derive(Debug, Clone, PartialEq)]
pub enum RunnerEvent {
    /// Always the first event; carries the session id to report finishes with.
    Registered {
        runner_id: Uuid,
    },
    Job(DispatchedJob),
}

impl RunnerEvent {
    /// Decode an SSE frame into an event. Unknown event names yield `None` so the protocol can
    /// grow without breaking older runners.
    pub fn decode(event: &str, data: &str) -> Result<Option<Self>, serde_json::Error> {
        #[derive(Deserialize)]
        struct Registered {
            runner_id: Uuid,
        }

        Ok(match event {
            "registered" => {
                let Registered { runner_id } = serde_json::from_str(data)?;
                Some(Self::Registered { runner_id })
            }
            "job" => Some(Self::Job(serde_json::from_str(data)?)),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_known_events_when_decoding_then_yields_them() {
        let id = Uuid::nil();

        let registered = RunnerEvent::decode("registered", &format!("{{\"runner_id\":\"{id}\"}}"))
            .unwrap()
            .unwrap();
        let job = RunnerEvent::decode(
            "job",
            &format!("{{\"id\":\"{id}\",\"job_name\":\"train\",\"input\":{{\"epochs\":2}}}}"),
        )
        .unwrap()
        .unwrap();

        assert_eq!(registered, RunnerEvent::Registered { runner_id: id });
        assert_eq!(
            job,
            RunnerEvent::Job(DispatchedJob {
                id,
                job_name: "train".to_string(),
                input: serde_json::json!({"epochs": 2}),
            })
        );
    }

    #[test]
    fn given_unknown_event_when_decoding_then_yields_none() {
        assert_eq!(RunnerEvent::decode("heartbeat", "{}").unwrap(), None);
    }

    #[test]
    fn given_malformed_data_when_decoding_then_errors() {
        assert!(RunnerEvent::decode("job", "not json").is_err());
    }
}
