pub mod client;
pub mod errors;
pub mod log;
pub mod metrics;
pub mod record;
pub mod schemas;

mod experiment;
mod http;
mod websocket;

pub use record::*;

pub mod command;

/// Core module that contains the main traits and types used in Burn Central.
mod core {
    use burn::prelude::Backend;
    use serde::de::DeserializeOwned;
    use serde::{Deserialize, Serialize};

    /// A module that contains everything related to experiments and tracking in Burn Central.
    pub mod tracking {
        pub trait Experiment {
            /// Get the name of the experiment.
            fn name(&self) -> &str;

            /// Get the description of the experiment.
            fn description(&self) -> &str;
        }
    }

    /// A module that contains the model-related traits and types.
    pub mod model {
        use crate::client::BurnCentralClient;
        use burn::prelude::Backend;

        /// A context that contains necessary information for operating with models managed by Burn Central.
        #[derive(Debug, Clone)]
        pub struct ModelContext<B: Backend> {
            pub client: Option<BurnCentralClient>,
            pub devices: Vec<B::Device>,
        }

        /// Interface exposed by models that can be used for prediction.
        pub trait Predict<B: Backend> {
            /// The input type for the prediction.
            type Input;

            /// The output type for the prediction.
            type Output;

            /// The error type for the prediction.
            type Error: std::fmt::Debug + std::fmt::Display;

            /// Predict the output for the given input.
            ///
            /// # Arguments
            ///
            /// * `input` - The input to predict on.
            ///
            /// * `context` - The context in which the model is being used, containing client and devices.
            ///
            /// # Returns
            ///
            /// A result containing the predicted output or an error if the prediction fails.
            fn predict(
                &self,
                input: Self::Input,
                context: ModelContext<B>,
            ) -> Result<Self::Output, Self::Error>;
        }

        pub trait ModelLoader<B: Backend> {
            type Model: Model<B>;
            /// Load a model from the given context.
            fn load_model(&self, context: ModelContext<B>) -> Result<Self::Model, String>;
        }

        impl<B, M, F> ModelLoader<B> for F
        where
            B: Backend,
            F: Fn(ModelContext<B>) -> Result<M, String>,
            M: Model<B>,
        {
            type Model = M;
            fn load_model(&self, context: ModelContext<B>) -> Result<M, String> {
                self(context)
            }
        }

        /// A trait that represents a model that can be used for prediction and can be loaded from a remote server.
        pub trait Model<B: Backend>: Predict<B> {
            /// Get the model name.
            fn model_name(&self) -> &str;
            /// Get the model description.
            fn model_description(&self) -> &str {
                ""
            }
        }
    }

    /// A module that contains the service-related traits and types to allow for the serving of models in user-defined applications.
    pub mod service {
        use crate::core::model::{Model, ModelContext, Predict};
        use axum::extract::{FromRequest, Request, State};
        use axum::response::IntoResponse;
        use axum::routing::post;
        use axum::Router;
        use burn::prelude::Backend;
        use serde::{Deserialize, Serialize};
        use std::sync::Arc;

        /// A trait that represents a routable service that can be used to serve api endpoints for models.
        pub trait Service<B: Backend> {
            /// Get the name of the service.
            fn name(&self) -> &str;

            /// Get the description of the service.
            fn description(&self) -> &str;

            /// Get the router for the service.
            fn into_router(self, context: ModelContext<B>) -> Router;
        }

        /// A service that wraps a model and provides an interface to serve it.
        pub struct ModelService<M> {
            pub model: M,
        }

        /// A trait that allows converting a type into a service that can be used to serve models.
        pub trait IntoService<B: Backend, S: Service<B>> {
            /// Convert the service into a router.
            fn into_service(self) -> S;
        }

        impl<B, M> IntoService<B, ModelService<M>> for M
        where
            B: Backend,
            M: Model<B> + 'static + Send + Sync,
            <M as Predict<B>>::Output: Serialize,
            <M as Predict<B>>::Input: for<'de> Deserialize<'de>,
        {
            fn into_service(self) -> ModelService<M> {
                ModelService::<M> { model: self }
            }
        }

        impl<B, M> Service<B> for ModelService<M>
        where
            B: Backend,
            M: Model<B> + 'static + Sync + Send,
            M::Input: for<'de> Deserialize<'de>,
            M::Output: Serialize,
        {
            fn name(&self) -> &str {
                self.model.model_name()
            }

            fn description(&self) -> &str {
                self.model.model_description()
            }

            fn into_router(self, context: ModelContext<B>) -> Router {
                use axum::Router;
                use axum::routing::get;

                async fn predict<B, M>(
                    state: State<ServiceStoreState<B, ModelService<M>>>,
                    req: Request,
                ) -> Result<axum::response::Response, axum::response::Response>
                where
                    B: Backend,
                    M: Model<B> + 'static + Sync + Send,
                    M::Input: for<'de> Deserialize<'de>,
                    M::Output: Serialize,
                {
                    let input = axum::extract::Json::from_request(req, &())
                        .await
                        .map_err(|e| e.into_response())?;
                    let context = (*state.context).clone();
                    let output = state.service.model.predict(input.0, context);
                    match output {
                        Ok(result) => Ok(axum::response::Json(result).into_response()),
                        Err(err) => Err(axum::response::Response::builder()
                            .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                            .body(axum::body::Body::from(err.to_string()))
                            .unwrap()),
                    }
                }

                Router::new()
                    .route("/", get(|| async { "Service is running" }))
                    .route("/model_name", get(|State(state): State<ServiceStoreState<B, ModelService<M>>>| {
                        async move { state.service.name().to_string() }
                    }))
                    .route("/model_description", get(|State(state): State<ServiceStoreState<B, ModelService<M>>>| {
                        async move { state.service.description().to_string() }
                    }))
                    .route(
                        "/predict",
                        post(predict),
                    )
                    .with_state(ServiceStoreState::new(self, context))
            }
        }

        pub struct ServiceStoreState<B: Backend, S: Service<B>> {
            pub service: Arc<S>,
            pub context: Arc<ModelContext<B>>,
        }

        /// Manual implementation of Clone to prevent adding bounds to the generic type parameters.
        impl<B: Backend, S: Service<B>> Clone for ServiceStoreState<B, S> {
            fn clone(&self) -> Self {
                Self {
                    service: Arc::clone(&self.service),
                    context: Arc::clone(&self.context),
                }
            }
        }

        impl<B: Backend, S: Service<B>> ServiceStoreState<B, S> {
            pub fn new(service: S, context: ModelContext<B>) -> Self {
                Self {
                    service: Arc::new(service),
                    context: Arc::new(context),
                }
            }
        }
    }

    /// A module that contains tests for the Rust API of burn-central-client.
    #[cfg(test)]
    mod ui_tests {
        use crate::client::{BurnCentralClient, BurnCentralClientConfig, BurnCentralCredentials};
        use crate::core::model::{Model, ModelContext, ModelLoader, Predict};
        use crate::core::service::{Service, ServiceStoreState};
        use crate::core::ui_tests::nn::{InputSchema, TestModel, TestModelLoader};
        use crate::schemas::ProjectPath;
        use axum::extract::{FromRequest, Request, State};
        use axum::http;
        use axum::response::IntoResponse;
        use axum::routing::get;
        use burn::backend::ndarray::NdArrayDevice;
        use burn::prelude::Backend;
        use http_body_util::BodyExt;
        use serde::{Deserialize, Serialize};
        use tower::util::ServiceExt;

        mod nn {
            use crate::core::model::{Model, ModelContext, ModelLoader, Predict};
            use crate::RemoteRecorder;
            use burn::module::Module;
            use burn::prelude::Backend;
            use burn::record::FullPrecisionSettings;
            use burn::record::Recorder;
            use serde::{Deserialize, Serialize};

            #[derive(Deserialize, Serialize, Debug)]
            pub struct InputSchema {
                pub hello: String,
            }

            #[derive(Serialize, Deserialize, Debug)]
            pub struct OutputSchema {
                pub message: String,
            }

            #[derive(Module, Debug)]
            pub struct InnerModule<B: Backend> {
                inner_param: u32,
                _phantom: std::marker::PhantomData<B>,
            }

            impl<B: Backend> InnerModule<B> {
                pub fn init(_device: &B::Device) -> Self {
                    Self {
                        inner_param: 24,
                        _phantom: Default::default(),
                    }
                }
            }

            #[derive(Module, Debug /* Model */)]
            // #[model(
            //      name = "testmodel",
            //      description = "A test model for Burn Central"
            // )]
            pub struct TestModel<B: Backend> {
                param: u32,
                inner: InnerModule<B>,
            }

            // Generated code for the model trait
            impl<B: Backend> Model<B> for TestModel<B> {
                fn model_name(&self) -> &str {
                    "testmodel"
                }

                fn model_description(&self) -> &str {
                    "A test model for Burn Central"
                }
            }

            // #[model_impl]
            impl<B: Backend> TestModel<B> {
                // #[init]
                pub fn init(device: &B::Device) -> Self {
                    Self {
                        param: 42,
                        inner: InnerModule::init(device),
                    }
                }
                // other methods could be added here perhaps for things like inference logic

                // #[predict]
                pub fn test_inference(
                    &self,
                    input: InputSchema,
                    context: ModelContext<B>,
                ) -> Result<OutputSchema, String> {
                    // Here we would implement the prediction logic
                    println!(
                        "Predict called with input: {:?} and {:?}",
                        input, context.devices
                    );

                    let message = format!("Hello, {}", input.hello);

                    Ok(OutputSchema { message })
                }
            }

            // Generated code for the model loader
            pub struct TestModelLoader;
            // Generated code for the model loader trait
            impl<B: Backend> ModelLoader<B> for TestModelLoader {
                type Model = TestModel<B>;

                // Boilerplate code to load a model from the context
                fn load_model(&self, context: ModelContext<B>) -> Result<Self::Model, String> {
                    let device = context
                        .devices
                        .first()
                        .ok_or("No device found in the context")?;

                    let rec = RemoteRecorder::<FullPrecisionSettings>::final_model(
                        context.client.expect("Client should be present"),
                    );
                    let record = rec
                        .load("mnist_model".parse().unwrap(), device)
                        .map_err(|e| e.to_string())?;

                    // Initialization logic for the model
                    let model = TestModel::<B>::init(device).load_record(record);

                    Ok(model)
                }
            }

            // Generated code for the predict trait
            impl<B: Backend> Predict<B> for TestModel<B> {
                type Input = InputSchema;
                type Output = OutputSchema;
                type Error = String;

                fn predict(
                    &self,
                    input: Self::Input,
                    context: ModelContext<B>,
                ) -> Result<Self::Output, String> {
                    self.test_inference(input, context)
                }
            }
        }

        #[test]
        fn test_load_model() {
            type B = burn::backend::NdArray;
            type D = burn::backend::ndarray::NdArrayDevice;

            let client = BurnCentralClientConfig::builder(
                BurnCentralCredentials::new("a".to_string()),
                ProjectPath::try_from("test/test".to_string()).unwrap(),
            )
            .build();
            let client = BurnCentralClient::create(client).unwrap();

            let devices = vec![D::default()];

            let context: ModelContext<_> = ModelContext {
                client: None,
                devices,
            };

            let loader = TestModelLoader;
            let loaded_model: TestModel<B> =
                loader.load_model(context).expect("Model should be loaded");

            assert_eq!(loaded_model.model_name(), "MNISTModel");
        }

        #[tokio::test]
        async fn test_service_router() {
            use crate::core::service::Service;
            use axum::Router;

            struct LoggedRequest {
                method: String,
                result: Result<String, String>,
                timestamp: std::time::SystemTime,
            }

            // #[derive(Service)]
            // #[service(
            //     name = "TestService",
            //     description = "A test service for Burn Central"
            // )]
            struct TestService<B: Backend> {
                model: TestModel<B>,
                logged_requests: tokio::sync::RwLock<Vec<LoggedRequest>>,
            }

            #[derive(Deserialize, Serialize, Debug)]
            struct MyServiceInputSchema {
                prediction_type: String,
                payload: String,
            }

            #[derive(Serialize, Deserialize, Debug)]
            struct MyServiceOutputSchema {
                result: String,
            }

            #[derive(Serialize, Deserialize, Debug)]
            struct MyServiceMetrics {
                total_requests: u32,
                successful_requests: u32,
                failed_requests: u32,
            }

            // #[service_impl]
            impl<B: Backend> TestService<B> {
                pub fn new(model: TestModel<B>) -> Self {
                    Self {
                        model,
                        logged_requests: Vec::new().into(),
                    }
                }

                // #[api(route = "/v1/predict", method = "POST")]
                pub async fn predict_something(
                    &self,
                    input: MyServiceInputSchema,
                    context: ModelContext<B>,
                ) -> Result<MyServiceOutputSchema, String> {
                    // Here we would implement the prediction logic
                    println!("Predict called with input: {:?}", input);
                    let input = InputSchema {
                        hello: input.payload,
                    };
                    let output = self.model.predict(input, context)?;
                    let result = MyServiceOutputSchema {
                        result: output.message,
                    };

                    // Log the request
                    let logged_request = LoggedRequest {
                        method: module_path!().to_string(),
                        result: Ok(result.result.clone()),
                        timestamp: std::time::SystemTime::now(),
                    };
                    self.logged_requests.write().await.push(logged_request);

                    Ok(result)
                }

                // #[api(route = "/metrics", method = "GET")]
                pub async fn metrics(
                    &self,
                    _context: ModelContext<B>,
                ) -> Result<MyServiceMetrics, String> {
                    // This is just a placeholder for the metadata metrics endpoint
                    let logged_requests = self.logged_requests.read().await;
                    let num_requests = logged_requests.len() as u32;
                    let successful_requests =
                        logged_requests.iter().filter(|r| r.result.is_ok()).count() as u32;
                    let failed_requests = num_requests - successful_requests;
                    let metrics = MyServiceMetrics {
                        total_requests: num_requests,
                        successful_requests,
                        failed_requests,
                    };
                    Ok(metrics)
                }
            }

            // Generated code for the service trait
            impl<B: Backend> Service<B> for TestService<B> {
                fn name(&self) -> &str {
                    "TestService"
                }

                fn description(&self) -> &str {
                    "A test service for Burn Central"
                }

                fn into_router(self, context: ModelContext<B>) -> Router {
                    use axum::Json;
                    use axum::routing::Router;
                    use axum::routing::post;

                    async fn predict_something_handler<B: Backend>(
                        State(state): State<ServiceStoreState<B, TestService<B>>>,
                        req: Request,
                    ) -> Result<axum::response::Response, axum::response::Response>
                    {
                        println!("Received request for prediction");
                        // Here we would call the predict method
                        let res = state
                            .service
                            .predict_something(
                                axum::Json::from_request(req, &())
                                    .await
                                    .map_err(|e| e.into_response())?
                                    .0,
                                (*state.context).clone(),
                            )
                            .await;
                        match res {
                            Ok(output) => Ok(Json(output).into_response()),
                            Err(err) => Err(axum::response::Response::builder()
                                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                                .body(axum::body::Body::from(err))
                                .unwrap()),
                        }
                    }

                    async fn metrics_handler<B: Backend>(
                        State(state): State<ServiceStoreState<B, TestService<B>>>,
                    ) -> Result<axum::response::Response, axum::response::Response>
                    {
                        // Here we would call the metrics method
                        let res = state.service.metrics((*state.context).clone()).await;
                        match res {
                            Ok(metrics) => Ok(Json(metrics).into_response()),
                            Err(err) => Err(axum::response::Response::builder()
                                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                                .body(axum::body::Body::from(err))
                                .unwrap()),
                        }
                    }

                    let router = Router::new()
                        // for each method in the service annotated with `#[api]` we generate a route
                        .route("/v1/predict", post(predict_something_handler))
                        .route("/metrics", get(metrics_handler))
                        .with_state(ServiceStoreState::new(self, context));
                    router
                }
            }

            type BackendImpl = burn::backend::NdArray;

            let client = BurnCentralClientConfig::builder(
                BurnCentralCredentials::new("6a18e7be-d027-45f0-a1e9-f0c67693aeb7".to_string()),
                ProjectPath::try_from("jwric/default".to_string()).unwrap(),
            )
            .build();
            let client =
                tokio::task::spawn_blocking(move || BurnCentralClient::create(client).unwrap())
                    .await
                    .unwrap();

            let devices = vec![NdArrayDevice::default()];

            let context: ModelContext<BackendImpl> = ModelContext {
                client: Some(client),
                devices,
            };
            // let model_loader = TestModelLoader;
            // let model: TestModel<BackendImpl> = model_loader
            //     .load_model(context)
            //     .expect("Model should be loaded");
            let model = TestModel::<BackendImpl>::init(&NdArrayDevice::default());
            let service = TestService::<BackendImpl>::new(model);
            let service_router = service.into_router(context);

            let input = MyServiceInputSchema {
                prediction_type: "test".to_string(),
                payload: "world".to_string(),
            };
            let response = service_router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(http::Method::POST)
                        .uri("/v1/predict")
                        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                        .body(axum::body::Body::from(
                            serde_json::to_vec(&input).expect("Failed to serialize input schema"),
                        ))
                        .unwrap(),
                )
                .await
                .expect("Failed to call the service");

            assert_eq!(response.status(), axum::http::StatusCode::OK);
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let output: MyServiceOutputSchema =
                serde_json::from_slice(&body).expect("Failed to parse response body");
            assert_eq!(output.result, "Hello, world");
            // Test the service name and description

            let response = service_router
                .oneshot(
                    Request::builder()
                        .method(http::Method::GET)
                        .uri("/metrics")
                        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .expect("Failed to call the service");

            assert_eq!(response.status(), axum::http::StatusCode::OK);
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let metrics: MyServiceMetrics =
                serde_json::from_slice(&body).expect("Failed to parse response body");
            assert_eq!(metrics.total_requests, 1);
            assert_eq!(metrics.successful_requests, 1);
            assert_eq!(metrics.failed_requests, 0);
        }

        #[tokio::test]
        async fn test_model_service() {
            use crate::core::service::IntoService;
            use crate::core::ui_tests::nn::TestModel;
            use burn::backend::ndarray::NdArrayDevice;

            type BackendImpl = burn::backend::NdArray;

            let client = BurnCentralClientConfig::builder(
                BurnCentralCredentials::new("6a18e7be-d027-45f0-a1e9-f0c67693aeb7".to_string()),
                ProjectPath::try_from("jwric/default".to_string()).unwrap(),
            )
            .build();
            let client =
                tokio::task::spawn_blocking(move || BurnCentralClient::create(client).unwrap())
                    .await
                    .unwrap();

            let devices = vec![NdArrayDevice::default()];

            let context: ModelContext<BackendImpl> = ModelContext {
                client: Some(client),
                devices,
            };

            // let model_loader = TestModelLoader;
            // let model: TestModel<BackendImpl> = model_loader.load_model(context).expect("Model should be loaded");
            let model = TestModel::<BackendImpl>::init(&NdArrayDevice::default());

            let service = model.into_service();

            let service_router = service.into_router(context);

            let input = nn::InputSchema {
                hello: "world".to_string(),
            };
            let response = service_router
                .oneshot(
                    Request::builder()
                        .method(http::Method::POST)
                        .uri("/predict")
                        .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
                        .body(axum::body::Body::from(
                            serde_json::to_vec(&input).expect("Failed to serialize input schema"),
                        ))
                        .unwrap(),
                )
                .await
                .expect("Failed to call the service");

            assert_eq!(response.status(), axum::http::StatusCode::OK);
            let body = response.into_body().collect().await.unwrap().to_bytes();
            let output: nn::OutputSchema =
                serde_json::from_slice(&body).expect("Failed to parse response body");
            assert_eq!(output.message, "Hello, world");
        }
    }
}
