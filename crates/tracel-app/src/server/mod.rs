mod error;

pub use error::ServerError;

use crate::{job::Job, job_register::JobRegister, mapper::JsonMapper};
use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
};
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;

pub struct Server {
    register: JobRegister,
    host: String,
    port: u16,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    pub fn new() -> Self {
        Self {
            register: JobRegister::new(),
            host: "0.0.0.0".to_string(),
            port: 3000,
        }
    }

    pub fn host(mut self, host: &str) -> Self {
        self.host = host.to_string();
        self
    }

    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn register<J, I, O>(mut self, job: J) -> Self
    where
        J: Job<I, O> + Send + Sync + 'static,
        I: DeserializeOwned + Send + Sync + 'static,
        O: 'static,
    {
        self.register = self.register.register(job, JsonMapper::new());
        self
    }

    pub fn register_with_default<J, I, O>(mut self, job: J, default: I) -> Self
    where
        J: Job<I, O> + Send + Sync + 'static,
        I: DeserializeOwned + Serialize + Send + Sync + 'static,
        O: 'static,
    {
        self.register = self
            .register
            .register(job, JsonMapper::with_default(default));
        self
    }

    pub fn run(self) -> Result<(), ServerError> {
        let addr = format!("{}:{}", self.host, self.port);
        let state = Arc::new(self.register);

        let app = Router::new()
            .route("/{job_name}", post(run_job))
            .with_state(state);

        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(async {
                let listener = tokio::net::TcpListener::bind(&addr).await?;
                axum::serve(listener, app).await?;
                Ok(())
            })
    }
}

async fn run_job(
    State(register): State<Arc<JobRegister>>,
    Path(job_name): Path<String>,
    body: String,
) -> impl IntoResponse {
    if !register.has_job(&job_name) {
        return (
            StatusCode::NOT_FOUND,
            format!(
                "unknown job '{}'. Available: {}",
                job_name,
                register.job_names().join(", ")
            ),
        );
    }

    let input = match register.validate(&job_name, &body) {
        Ok(input) => input,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("Invalid configuration for job '{}': {}", job_name, e),
            );
        }
    };

    let response_name = job_name.clone();

    tokio::task::spawn_blocking(move || {
        if let Err(e) = register.run(&job_name, input) {
            eprintln!("Job '{job_name}' failed: {e}");
        }
    });

    (
        StatusCode::OK,
        format!("Job '{response_name}' has started running"),
    )
}
