mod error;
/// Request-body decoders for server routes.
pub mod mapper;
mod route;

pub use error::ServerError;
pub use mapper::{BodyMapper, JsonBody};
pub use route::{IntoServerRoute, ServerRoute};

use axum::{
    Router,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use route::MAX_BODY_BYTES;
use std::collections::HashMap;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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

    /// Register a capability job (experiment, inference, ...) at `POST /{name}`, decoding its input
    /// with `mapper`.
    ///
    /// The same call works for any job type that implements [`IntoServerRoute`]: experiments respond
    /// fire-and-forget, inference streams its outputs over SSE.
    pub fn register<T, I>(self, job: T, mapper: impl BodyMapper<I> + 'static) -> Self
    where
        T: IntoServerRoute<I>,
    {
        self.route_boxed(job.into_server_route(Arc::new(mapper)))
    }

    pub async fn run_async(self) -> Result<(), ServerError> {
        let addr = format!("{}:{}", self.host, self.port);
        let state = Arc::new(self.routes);

        let app = Router::new()
            .route("/{name}", post(dispatch))
            .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
            .with_state(state);

        let _ = tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(tracel_experiment::integration::tracing::tracing_log_layer())
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .try_init();

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
