mod error;
mod route;

pub use error::ServerError;
pub use route::{IntoServerRoute, ServerRoute};

use axum::{
    Router,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use route::{ExperimentRoute, MAX_BODY_BYTES};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::sync::Arc;
use tracel_experiment::ExperimentJob;

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

    /// Register a bespoke [`ServerRoute`]. [`register`](Self::register) builds on this for capability
    /// jobs; use this directly only for a custom route.
    pub fn route<R>(self, route: R) -> Self
    where
        R: ServerRoute + 'static,
    {
        self.route_boxed(Box::new(route))
    }

    fn route_boxed(mut self, route: Box<dyn ServerRoute>) -> Self {
        let name = route.name().to_string();
        if self.routes.contains_key(&name) {
            panic!("route '{name}' is already registered");
        }
        self.routes.insert(name, route);
        self
    }

    /// Register a capability job (experiment, inference, ...) at `POST /{name}`.
    ///
    /// The same call works for any job type that implements [`IntoServerRoute`]: experiments respond
    /// fire-and-forget, inference streams its outputs over SSE.
    pub fn register<T>(self, job: T) -> Self
    where
        T: IntoServerRoute,
    {
        self.route_boxed(job.into_server_route())
    }

    /// Register an experiment job with a default config merged into request bodies.
    pub fn register_with_default<I, O>(self, job: ExperimentJob<I, O>, default: I) -> Self
    where
        I: DeserializeOwned + Serialize + Send + 'static,
        O: 'static,
    {
        self.route_boxed(Box::new(ExperimentRoute::with_default(job, default)))
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
    request: Request,
) -> Response {
    match routes.get(&name) {
        Some(route) => route.handle(request.into_body()).await,
        None => (StatusCode::NOT_FOUND, format!("unknown route '{name}'")).into_response(),
    }
}
