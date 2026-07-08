mod error;
mod route;

pub use error::ServerError;
pub use route::{ExperimentRoute, InferenceRoute, ServerRoute};

use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::sync::Arc;
use tracel_experiment::ExperimentJob;
use tracel_inference::InferenceJob;

/// Maximum request body size accepted (10 MiB).
const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

type Routes = HashMap<String, Box<dyn ServerRoute>>;

pub struct Server {
    routes: Routes,
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
            routes: HashMap::new(),
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

    /// Register any [`ServerRoute`]. Capability-specific helpers build on this.
    pub fn route<R>(mut self, route: R) -> Self
    where
        R: ServerRoute + 'static,
    {
        let name = route.name().to_string();
        if self.routes.contains_key(&name) {
            panic!("route '{name}' is already registered");
        }
        self.routes.insert(name, Box::new(route));
        self
    }

    /// Register an experiment job at `POST /{name}` (fire-and-forget).
    pub fn register<I, O>(self, job: ExperimentJob<I, O>) -> Self
    where
        I: DeserializeOwned + Send + 'static,
        O: 'static,
    {
        self.route(ExperimentRoute::new(job))
    }

    /// Register an experiment job with a default config merged into request bodies.
    pub fn register_with_default<I, O>(self, job: ExperimentJob<I, O>, default: I) -> Self
    where
        I: DeserializeOwned + Serialize + Send + 'static,
        O: 'static,
    {
        self.route(ExperimentRoute::with_default(job, default))
    }

    /// Register a streaming inference job at `POST /{name}`, served over SSE.
    pub fn register_inference<I, O>(self, job: InferenceJob<I, O>) -> Self
    where
        I: DeserializeOwned + Send + 'static,
        O: Serialize + Send + Sync + 'static,
    {
        self.route(InferenceRoute::new(job))
    }

    pub async fn run_async(self) -> Result<(), ServerError> {
        let addr = format!("{}:{}", self.host, self.port);
        let state = Arc::new(self.routes);

        let app = Router::new()
            .route("/{name}", post(dispatch))
            .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
            .with_state(state);

        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .try_init();
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!(
            "Server listening on http://localhost:{}",
            listener.local_addr()?.port()
        );
        axum::serve(listener, app).await?;
        Ok(())
    }

    pub fn run(self) -> Result<(), ServerError> {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(self.run_async())
    }
}

async fn dispatch(
    State(routes): State<Arc<Routes>>,
    Path(name): Path<String>,
    body: Bytes,
) -> Response {
    match routes.get(&name) {
        Some(route) => route.handle(body),
        None => (StatusCode::NOT_FOUND, format!("unknown route '{name}'")).into_response(),
    }
}
