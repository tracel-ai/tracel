//! How the server runs a job from a request: its inputs from the request body, and its outputs
//! and how it ended as Server-Sent Events.

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Context, Poll};

use axum::{
    body::Body,
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::oneshot;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tracel_experiment::CancelToken;
use tracel_inference::{OutputWriter, OutputWriterError};
use tracel_job::{ReportedExperiment, RunStatus};

use super::input::feed;
use crate::panics::catch_panic;
use crate::{BoxError, ExperimentReporter, Job, JobContext, JobInput, PreparedJob};

pub const MAX_BODY_BYTES: usize = 10 * 1024 * 1024 * 1024;

type Events = tokio::sync::mpsc::Sender<Result<Event, Infallible>>;

/// Runs `job` with the inputs of the request body `body`, answering with the job's events as it
/// runs, as [`Server`](super::Server) describes them, or with `400 Bad Request` when the job
/// rejects its input before it runs.
pub async fn run(job: Arc<dyn Job>, body: Body) -> Response {
    let (inputs, received) = mpsc::channel();
    tokio::spawn(feed(body, inputs));

    let (events, sent) = tokio::sync::mpsc::channel(64);
    let cancel_token = CancelToken::new();
    let stream = EventStream {
        events: ReceiverStream::new(sent),
        cancel_token: cancel_token.clone(),
        ended: false,
    };

    // A job that takes one input reads it as it is prepared, waiting for the body to end, so the
    // job is prepared, then run, on the blocking pool.
    let (prepared, started) = oneshot::channel();
    tokio::task::spawn_blocking(move || {
        match job.prepare(JobInput::Stream(Box::new(received.into_iter()))) {
            Ok(job) => {
                if prepared.send(Ok(())).is_ok() {
                    run_prepared(job, events, cancel_token);
                }
            }
            Err(error) => {
                let _ = prepared.send(Err(error));
            }
        }
    });

    match started.await {
        Ok(Ok(())) => Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response(),
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "the job panicked before it ran",
        )
            .into_response(),
    }
}

/// Runs `job`, asked to stop once `cancel_token` is cancelled, and sends its events: its outputs
/// and errors as it writes them, the experiment it records once it records it, and how it ended.
fn run_prepared(job: PreparedJob, events: Events, cancel_token: CancelToken) {
    let experiment = Arc::new(Mutex::new(None));
    let reporter = ExperimentReporter::new({
        let (experiment, events) = (experiment.clone(), events.clone());
        move |reported: ReportedExperiment| {
            let _ = events.blocking_send(Ok(json_event("experiment", &reported)));
            *experiment.lock().unwrap() = Some(reported);
        }
    });
    let context = JobContext::new(cancel_token).with_reporter(reporter);
    let output = EventOutput {
        events: events.clone(),
    };

    let (status, error) = match catch_panic(|| job.run(output, context)) {
        Ok(()) => (RunStatus::Completed, None),
        Err(error) => (RunStatus::Failed, Some(error.to_string())),
    };
    let done = Done {
        status,
        error,
        experiment: experiment.lock().unwrap().take(),
    };
    let _ = events.blocking_send(Ok(json_event("done", &done)));
}

/// How a job ended, as the `done` event gives it: completed, or failed with its error, and the
/// experiment it recorded, if any.
#[derive(Serialize)]
struct Done {
    status: RunStatus,
    error: Option<String>,
    experiment: Option<ReportedExperiment>,
}

/// The event `name`, whose data is `data` as JSON.
fn json_event(name: &str, data: &impl Serialize) -> Event {
    Event::default()
        .event(name)
        .json_data(data)
        .unwrap_or_else(|error| error_event(error.to_string()))
}

fn error_event(message: String) -> Event {
    Event::default().event("error").data(message)
}

/// Sends a job's outputs as events as it writes them: each output as a `data:` frame, and each
/// error as an `error` event. A failed send means the client disconnected, reported as
/// [`OutputWriterError::Cancelled`] so the job stops writing.
struct EventOutput {
    events: Events,
}

impl EventOutput {
    fn send(&self, event: Event) -> Result<(), OutputWriterError> {
        self.events
            .blocking_send(Ok(event))
            .map_err(|_| OutputWriterError::Cancelled)
    }
}

impl OutputWriter<Value> for EventOutput {
    fn write(&self, output: Value) -> Result<(), OutputWriterError> {
        self.send(Event::default().data(output.to_string()))
    }

    fn error(&self, error: BoxError) -> Result<(), OutputWriterError> {
        self.send(error_event(error.to_string()))
    }

    fn finish(&self, _duration: std::time::Duration) {}
}

/// A job's events, as the response streams them. Dropped before the job's last event, as when the
/// client disconnects, it asks the job to stop.
struct EventStream {
    events: ReceiverStream<Result<Event, Infallible>>,
    cancel_token: CancelToken,
    ended: bool,
}

impl Stream for EventStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let next = Pin::new(&mut self.events).poll_next(cx);
        if let Poll::Ready(None) = next {
            self.ended = true;
        }
        next
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        if !self.ended {
            self.cancel_token.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use serde_json::json;
    use tracel_experiment::local::LocalExperiments;
    use tracel_experiment::{ExperimentRun, Experiments};
    use tracel_inference::{
        InferenceInput, InferenceModule, InferenceOutput, InferenceSession, NoopInferenceProvider,
    };
    use tracel_job::JobDefinition;

    use super::*;
    use crate::IntoJob;
    use crate::mapper::JsonMapper;
    use crate::test_support::NeverRuns;

    /// A job that takes one input and runs `run` in its context.
    struct Fake {
        definition: JobDefinition,
        run: Arc<dyn Fn(JobContext) -> Result<(), BoxError> + Send + Sync>,
    }

    impl Job for Fake {
        fn definition(&self) -> &JobDefinition {
            &self.definition
        }

        fn prepare(&self, input: JobInput) -> Result<PreparedJob, BoxError> {
            input.one()?;
            let run = self.run.clone();
            Ok(PreparedJob::new(move |_output, context| run(context)))
        }
    }

    fn fake(
        run: impl Fn(JobContext) -> Result<(), BoxError> + Send + Sync + 'static,
    ) -> Arc<dyn Job> {
        Arc::new(Fake {
            definition: JobDefinition {
                name: "fake".to_string(),
                description: None,
                input_schema: None,
                input_example: None,
            },
            run: Arc::new(run),
        })
    }

    /// A job recording experiments under `dir`, which records the input it runs with in `ran` and
    /// fails when it runs for no epochs.
    fn train(dir: &Path, ran: &Arc<Mutex<Vec<Value>>>) -> Arc<dyn Job> {
        let ran = ran.clone();
        Arc::from(
            Experiments::new(Arc::new(LocalExperiments::new(dir)))
                .create("train", move |_run: &ExperimentRun, config: Value| {
                    ran.lock().unwrap().push(config.clone());
                    if config["epochs"] == 0 {
                        return Err("no epochs".into());
                    }
                    Ok(())
                })
                .into_job(JsonMapper::with_default(json!({"epochs": 10}))),
        )
    }

    /// A job that answers each text with its words.
    fn words() -> Arc<dyn Job> {
        Arc::from(
            InferenceModule::new(Arc::new(NoopInferenceProvider::new()))
                .create(
                    "words",
                    |_session: &InferenceSession,
                     input: InferenceInput<String>,
                     output: InferenceOutput<String>| {
                        for text in input {
                            for word in text.split_whitespace() {
                                let _ = output.write(word.to_string());
                            }
                        }
                    },
                )
                .into_job(JsonMapper::<String>::new()),
        )
    }

    /// The events of `response`, read to its end: each event's name, `message` for an output, and
    /// its data, as JSON when it is JSON.
    async fn events(response: Response) -> Vec<(String, Value)> {
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(body.to_vec())
            .unwrap()
            .split("\n\n")
            .filter(|event| !event.is_empty())
            .map(|event| {
                let mut name = "message".to_string();
                let mut data = Vec::new();
                for line in event.lines() {
                    if let Some(event) = line.strip_prefix("event:") {
                        name = event.trim_start().to_string();
                    } else if let Some(line) = line.strip_prefix("data:") {
                        data.push(line.strip_prefix(' ').unwrap_or(line));
                    }
                }
                let data = data.join("\n");
                let data = serde_json::from_str(&data).unwrap_or(Value::String(data));
                (name, data)
            })
            .collect()
    }

    fn event(name: &str, data: Value) -> (String, Value) {
        (name.to_string(), data)
    }

    async fn rejection(response: Response) -> String {
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(body.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn a_job_that_records_an_experiment_sends_it_then_how_the_job_ended() {
        let dir = tempfile::tempdir().unwrap();
        let ran = Arc::default();

        let job = train(dir.path(), &ran);
        let completed = events(run(job.clone(), Body::from(r#"{"epochs": 3}"#)).await).await;
        let failed = events(run(job, Body::from(r#"{"epochs": 0}"#)).await).await;

        let runs = dir.path().canonicalize().unwrap();
        let experiment =
            |num: u64| json!({"num": num, "url": null, "dir": runs.join(format!("train/{num}"))});
        assert_eq!(
            completed,
            [
                event("experiment", experiment(1)),
                event(
                    "done",
                    json!({"status": "completed", "error": null, "experiment": experiment(1)})
                ),
            ]
        );
        assert_eq!(
            failed,
            [
                event("experiment", experiment(2)),
                event(
                    "done",
                    json!({"status": "failed", "error": "no epochs", "experiment": experiment(2)})
                ),
            ]
        );
        assert_eq!(
            *ran.lock().unwrap(),
            [json!({"epochs": 3}), json!({"epochs": 0})]
        );
    }

    #[tokio::test]
    async fn the_one_input_of_a_job_may_span_lines_and_an_empty_body_is_null() {
        let dir = tempfile::tempdir().unwrap();
        let ran = Arc::default();

        let job = train(dir.path(), &ran);
        for body in ["{\n  \"epochs\": 2\n}\n", ""] {
            events(run(job.clone(), Body::from(body)).await).await;
        }

        assert_eq!(
            *ran.lock().unwrap(),
            [json!({"epochs": 2}), json!({"epochs": 10})]
        );
    }

    #[tokio::test]
    async fn an_input_the_job_rejects_before_it_runs_is_a_bad_request() {
        let job: Arc<dyn Job> = Arc::from(
            Experiments::new(Arc::new(NeverRuns))
                .create("train", |_run: &ExperimentRun, _epochs: u32| Ok(()))
                .into_job(JsonMapper::with_default(10u32)),
        );

        for (body, reason) in [
            (r#""ten""#, "\"ten\""),
            ("3\n4\n", "the job takes one input, but was given more"),
            ("{\"epochs\":", "invalid JSON: "),
        ] {
            let rejection = rejection(run(job.clone(), Body::from(body)).await).await;

            assert!(rejection.contains(reason), "{rejection}");
        }
    }

    #[tokio::test]
    async fn a_job_that_takes_several_inputs_answers_each() {
        let answered = events(run(words(), Body::from("\"one two\"\n\"three\"\n")).await).await;
        let stopped = events(run(words(), Body::from("\"one\"\nnope\n\"two\"\n")).await).await;

        let done = event(
            "done",
            json!({"status": "completed", "error": null, "experiment": null}),
        );
        assert_eq!(
            answered,
            [
                event("message", json!("one")),
                event("message", json!("two")),
                event("message", json!("three")),
                done.clone(),
            ]
        );
        assert_eq!(stopped[0], event("message", json!("one")));
        assert!(
            matches!(&stopped[1], (name, Value::String(error))
                if name == "error" && error.starts_with("invalid JSON: ")),
            "{stopped:?}"
        );
        assert_eq!(stopped[2..], [done]);
    }

    #[tokio::test]
    async fn a_job_that_panics_ends_failed() {
        let panics = fake(|_context| panic!("kernel exploded"));

        let events = events(run(panics, Body::empty()).await).await;

        assert_eq!(
            events,
            [event(
                "done",
                json!({
                    "status": "failed",
                    "error": "the job panicked: kernel exploded",
                    "experiment": null
                })
            )]
        );
    }

    #[tokio::test]
    async fn a_client_that_disconnects_asks_the_job_to_stop() {
        let (stopped, asked) = mpsc::channel();
        let job = fake(move |context| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !context.cancel_token().is_cancelled() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = stopped.send(context.cancel_token().is_cancelled());
            Ok(())
        });

        drop(run(job, Body::empty()).await);

        assert_eq!(asked.recv_timeout(Duration::from_secs(20)), Ok(true));
    }

    #[tokio::test]
    async fn a_job_that_ends_is_not_asked_to_stop() {
        let cancel_token = Arc::new(Mutex::new(None));
        let job = fake({
            let cancel_token = cancel_token.clone();
            move |context| {
                *cancel_token.lock().unwrap() = Some(context.cancel_token().clone());
                Ok(())
            }
        });

        events(run(job, Body::empty()).await).await;

        let cancel_token = cancel_token.lock().unwrap().take().unwrap();
        assert!(!cancel_token.is_cancelled());
    }
}
