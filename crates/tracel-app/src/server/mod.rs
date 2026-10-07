mod error;
mod input;
mod request;

pub use error::ServerError;

use std::sync::Arc;

use axum::{
    RequestExt, Router,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{IntoJob, Job, JobRegistry};
use request::MAX_BODY_BYTES;

/// Serves every registered job over HTTP at `POST /{job_name}`.
///
/// The request body is the job's input: one JSON document per line (NDJSON), each handed to the
/// job once its line has arrived, or one document over several lines, such as a pretty-printed
/// one, handed over once the body has ended. An empty body is a `null` input. A job that takes
/// one input takes the body's one document; a job that takes several takes each as it arrives.
///
/// The response streams the job's events as Server-Sent Events while it runs:
///
/// | Event | Data | Sent |
/// | --- | --- | --- |
/// | `message` | an output, as JSON | for each output the job writes, as an unnamed event |
/// | `error` | the error, as text | for each error the job reports |
/// | `experiment` | the experiment, as JSON | when the job records an experiment |
/// | `done` | how the job ended, as JSON | once, when the job ends, as the last event |
///
/// ```text
/// event: experiment
/// data: {"num":4,"url":null,"dir":"/home/me/demo/runs/train/4"}
///
/// event: done
/// data: {"status":"completed","error":null,"experiment":{"num":4,"url":null,"dir":"/home/me/demo/runs/train/4"}}
/// ```
///
/// An experiment is given as a [`RunReport`](tracel_job::RunReport) links it: its `num`, and its
/// page on the console as `url` or, offline, its directory as `dir`. In `done`, `status` is
/// `completed` or `failed`, by what the job returned, `error` says why a job failed, and
/// `experiment` is the experiment the job recorded, or `null`. A job that takes several inputs
/// reports an input that is not JSON, or does not decode, as an `error` and takes no more.
///
/// An unknown job is answered with `404 Not Found`. An input the job rejects before it runs, such
/// as one that is not JSON or does not decode, or a second document for a job that takes one, is
/// answered with `400 Bad Request` and the reason. A client that disconnects asks the job to stop,
/// through the cancel token of its [`JobContext`](crate::JobContext).
pub struct Server {
    jobs: JobRegistry,
    host: String,
    port: u16,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    /// A server with no jobs, listening on `0.0.0.0:3000`.
    pub fn new() -> Self {
        Self {
            jobs: JobRegistry::new(),
            host: "0.0.0.0".to_string(),
            port: 3000,
        }
    }

    /// Sets the host to listen on.
    pub fn host(mut self, host: &str) -> Self {
        self.host = host.to_string();
        self
    }

    /// Sets the port to listen on.
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Registers a capability job (an experiment or an inference), decoding its input with
    /// `mapper`.
    ///
    /// # Panics
    ///
    /// When a job with the same name is already registered.
    pub fn register<J, M>(mut self, job: J, mapper: M) -> Self
    where
        J: IntoJob<M>,
    {
        self.jobs.add(job.into_job(mapper));
        self
    }

    /// Registers a job that implements [`Job`] directly.
    ///
    /// # Panics
    ///
    /// When a job with the same name is already registered.
    pub fn job(mut self, job: impl Job + 'static) -> Self {
        self.jobs.add(Box::new(job));
        self
    }

    /// Serves the registered jobs on the current Tokio runtime until the server stops.
    ///
    /// When `TRACEL_DESCRIBE` names a path, writes the definitions file there instead and returns
    /// without serving.
    pub async fn run_async(self) -> Result<(), ServerError> {
        if self.jobs.describe_from_env("server")? {
            return Ok(());
        }
        self.serve().await
    }

    /// Serves the registered jobs on a new Tokio runtime until the server stops.
    ///
    /// When `TRACEL_DESCRIBE` names a path, writes the definitions file there instead and returns
    /// without serving.
    pub fn run(self) -> Result<(), ServerError> {
        if self.jobs.describe_from_env("server")? {
            return Ok(());
        }
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(self.serve())
    }

    async fn serve(self) -> Result<(), ServerError> {
        let addr = format!("{}:{}", self.host, self.port);
        let app = Router::new()
            .route("/{name}", post(dispatch))
            .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
            .with_state(Arc::new(self.jobs));

        let _ = tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(tracel_experiment::integration::tracing::tracing_log_layer())
            .with(tracel_inference::integration::tracing::inference_log_layer())
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .try_init();

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!(
            "Server listening on http://localhost:{}",
            listener.local_addr()?.port()
        );
        axum::serve(listener, app).await?;
        Ok(())
    }
}

async fn dispatch(
    State(jobs): State<Arc<JobRegistry>>,
    Path(name): Path<String>,
    request: Request,
) -> Response {
    let Some(job) = jobs.get(&name) else {
        return (StatusCode::NOT_FOUND, format!("unknown job '{name}'")).into_response();
    };
    request::run(job, request.into_limited_body()).await
}

#[cfg(test)]
mod tests {
    use axum::body::Body;

    use super::*;

    #[tokio::test]
    async fn an_unknown_job_is_not_found() {
        let request = Request::new(Body::from("{}"));

        let response = dispatch(
            State(Arc::new(JobRegistry::new())),
            Path("train".to_string()),
            request,
        )
        .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body, "unknown job 'train'");
    }
}
