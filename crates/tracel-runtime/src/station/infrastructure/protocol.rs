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
    /// The job ended after the station asked it to stop, whatever it returned.
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinishJob {
    /// The session id received in the `registered` event.
    pub runner_id: Uuid,
    pub status: FinishStatus,
    /// Why the job failed, when it did, cancelled or not.
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
    /// The station asks the job `job_id` to stop.
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
        let cancel = RunnerEvent::decode("cancel", &format!("{{\"job_id\":\"{id}\"}}"))
            .unwrap()
            .unwrap();

        assert_eq!(registered, RunnerEvent::Registered { runner_id: id });
        assert_eq!(cancel, RunnerEvent::Cancel { job_id: id });
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
        assert!(RunnerEvent::decode("cancel", "{}").is_err());
    }

    #[test]
    fn a_registration_gives_each_job_its_name_description_and_input() {
        let definition = JobDefinition {
            name: "train".to_string(),
            description: Some("Train the model".to_string()),
            input_schema: Some(serde_json::json!({"type": "integer"})),
            input_example: Some(serde_json::json!(3)),
        };

        let register = RegisterRunner {
            name: Some("runner".to_string()),
            jobs: vec![RegisteredJob::from(&definition)],
        };

        assert_eq!(
            serde_json::to_value(register).unwrap(),
            serde_json::json!({
                "name": "runner",
                "jobs": [{
                    "name": "train",
                    "description": "Train the model",
                    "input_schema": {"type": "integer"},
                    "input_example": 3
                }]
            })
        );
    }

    #[test]
    fn a_finish_gives_its_status_in_snake_case() {
        let finish = |status| {
            serde_json::to_value(FinishJob {
                runner_id: Uuid::nil(),
                status,
                reason: None,
            })
            .unwrap()["status"]
                .clone()
        };

        assert_eq!(finish(FinishStatus::Completed), "completed");
        assert_eq!(finish(FinishStatus::Failed), "failed");
        assert_eq!(finish(FinishStatus::Cancelled), "cancelled");
    }
}
