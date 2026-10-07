mod error;
mod request;

pub use error::ServerError;

use std::sync::Arc;

use axum::{
    Router,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use tracel_job::JobKind;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{IntoJob, Job, JobRegistry};
use request::MAX_BODY_BYTES;

/// Serves every registered job over HTTP at `POST /{job_name}`.
///
/// The request body is the job's JSON input. An experiment takes one JSON document (an empty
/// body is no input) and the response returns once it has started. An inference takes NDJSON,
/// one input per line as it arrives, and streams its outputs back as Server-Sent Events.
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
    match job.definition().kind {
        JobKind::Experiment => request::start_experiment(job, request.into_body()).await,
        JobKind::Inference => request::stream_inference(job, request.into_body()),
    }
}
