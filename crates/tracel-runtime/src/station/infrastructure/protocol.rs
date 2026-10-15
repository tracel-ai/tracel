//! Wire types for the station's runner protocol.
//!
//! Mirrors the station's `/v1/runners/events` (registration + SSE signals) and
//! `/v1/jobs/{id}/finish` contracts.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::JobDefinition;

/// Registration body; the response to it is the SSE event stream.
#[derive(Debug, Clone, Serialize)]
pub struct RegisterRunner {
    /// Optional display label. Duplicates are allowed — identity is the session.
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub metadata: Map<String, Value>,
    /// The jobs this runner can execute.
    pub jobs: Vec<RegisteredJob>,
}

/// A job the runner can execute, as it registers it.
#[derive(Debug, Clone, Serialize)]
pub struct RegisteredJob {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Option<Value>,
    pub input_example: Option<Value>,
}

impl From<&JobDefinition> for RegisteredJob {
    fn from(definition: &JobDefinition) -> Self {
        Self {
            name: definition.name.clone(),
            description: definition.description.clone(),
            input_schema: definition.input_schema.clone(),
            input_example: definition.input_example.clone(),
        }
    }
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
    /// The station cancelled the job `job_id`.
    Cancel {
        job_id: Uuid,
    },
}

impl RunnerEvent {
    /// Decode an SSE frame into an event. Unknown event names yield `None` so the protocol can
    /// grow without breaking older runners.
    pub fn decode(event: &str, data: &str) -> Result<Option<Self>, serde_json::Error> {
        #[derive(Deserialize)]
        struct Registered {
            runner_id: Uuid,
        }

        #[derive(Deserialize)]
        struct Cancel {
            job_id: Uuid,
        }

        Ok(match event {
            "registered" => {
                let Registered { runner_id } = serde_json::from_str(data)?;
                Some(Self::Registered { runner_id })
            }
            "job" => Some(Self::Job(serde_json::from_str(data)?)),
            "cancel" => {
                let Cancel { job_id } = serde_json::from_str(data)?;
                Some(Self::Cancel { job_id })
            }
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_event_decodes_to_nothing() {
        assert_eq!(RunnerEvent::decode("heartbeat", "{}").unwrap(), None);
    }
}
