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

        use serde::{Deserialize, Serialize};

        /// A context that contains necessary information for operating with models managed by Burn Central.
        #[derive(Debug, Clone)]
        pub struct ModelContext<B: Backend> {
            pub client: Option<BurnCentralClient>,
            pub devices: Vec<B::Device>,
        }

        /// Interface exposed by models that can be used for prediction.
        pub trait Predict<B: Backend> {
            /// The input type for the prediction.
            type Input<'a>: for<'b> Deserialize<'b>;

            /// The output type for the prediction.
            type Output: Serialize;

            /// Predict the output for the given input.
            ///
            /// # Arguments
            ///
            /// * `input` - The input to predict on.
            ///
            /// # Returns
            ///
            /// The predicted output.
            fn predict(
                &self,
                input: Self::Input<'_>,
                context: ModelContext<B>,
            ) -> Result<Self::Output, String>;
        }

        // /// A trait that represents a model that can be loaded from an arbitrary source, such as a remote server.
        // pub trait Load<B: Backend> {
        //     fn load_from_context(self, context: ModelContext<B>) -> Result<Self, String>
        //     where
        //         Self: Sized;
        // }

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

        // impl<R, B> Load<B> for R
        // where
        //     R: burn::module::Module<B>,
        //     B: Backend,
        // {
        //     fn load_from_context(self, context: ModelContext<B>) -> Result<Self, String> {
        //         let device = context
        //             .devices
        //             .first()
        //             .ok_or("No device found in the context")?;
        //
        //         let rec =
        //             RemoteRecorder::<FullPrecisionSettings>::final_model(context.client.clone());
        //         let record = rec
        //             .load("".parse().unwrap(), device)
        //             .map_err(|e| e.to_string())?;
        //
        //         Ok(self.load_record(record))
        //     }
        // }
    }

    /// A module that contains the service-related traits and types to allow for the serving of models in user-defined applications.
    pub mod service {
        use std::println;
        use axum::Router;
        use axum::routing::post;
        use burn::prelude::Backend;

        /// A trait that represents a routable service that can be used to serve api endpoints for models.
        pub trait Service<B: Backend> {
            /// Get the name of the service.
            fn name(&self) -> &str;

            /// Get the description of the service.
            fn description(&self) -> &str;

            /// Get the router for the service.
            fn into_router(self) -> Router;
        }
    }

    /// A module that contains tests for the Rust API of burn-central-client.
    mod ui_tests {
        use std::sync::Arc;
        use axum::extract::{Request, State};
        use axum::response::IntoResponse;
        use burn::backend::ndarray::NdArrayDevice;
        use crate::client::{BurnCentralClient, BurnCentralClientConfig, BurnCentralCredentials};
        use crate::core::model::{Model, ModelContext, ModelLoader, Predict};
        use crate::core::ui_tests::nn::{InputSchema, OutputSchema, TestModel, TestModelLoader};
        use crate::schemas::ProjectPath;
        use burn::prelude::Backend;
        use crate::core::service::Service;

        mod nn {
            use crate::RemoteRecorder;
            use crate::core::model::{Model, ModelContext, ModelLoader, Predict};
            use burn::module::Module;
            use burn::prelude::Backend;
            use burn::record::FullPrecisionSettings;
            use burn::record::Recorder;
            use serde::{Deserialize, Serialize};

            #[derive(Deserialize, Debug)]
            pub struct InputSchema {
                hello: String,
            }

            #[derive(Serialize, Debug)]
            pub struct OutputSchema {
                message: String,
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
                    "MNISTModel"
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
                    _input: InputSchema,
                    _context: ModelContext<B>,
                ) -> Result<OutputSchema, String> {
                    // Here we would implement the prediction logic
                    println!("Predict called with input: {:?}", _input);

                    let message = format!("Hello, {}", _input.hello);

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
                type Input<'a> = InputSchema;
                type Output = OutputSchema;
                fn predict(
                    &self,
                    input: Self::Input<'_>,
                    context: ModelContext<B>,
                ) -> Result<Self::Output, String> {
                    self.test_inference(input, context)
                }
            }
        }
        fn test_load_model<B: Backend>() {
            let client = BurnCentralClientConfig::builder(
                BurnCentralCredentials::new("a".to_string()),
                ProjectPath::try_from("test/test".to_string()).unwrap(),
            )
                .build();
            let client = BurnCentralClient::create(client).unwrap();

            let devices = vec![B::Device::default()];

            let context: ModelContext<_> = ModelContext { client: None, devices };

            let loader = TestModelLoader;
            let loaded_model: TestModel<B> =
                loader.load_model(context).expect("Model should be loaded");

            assert_eq!(loaded_model.model_name(), "MNISTModel");
        }

        #[tokio::test]
        async fn test_service_router() {
            use crate::core::service::Service;
            use axum::Router;

            #[derive(/* Service */)]
            // #[service(
            //     name = "TestService",
            //     description = "A test service for Burn Central"
            // )]
            struct TestService<B: Backend> {
                model: TestModel<B>,
            }

            // #[service_impl]
            impl<B: Backend> TestService<B> {
                pub fn new(model: TestModel<B>) -> Self {
                    Self {
                        model,
                    }
                }

                // #[api(route = "/predict", method = "POST")]
                pub fn predict(
                    &self,
                    input: InputSchema,
                    context: ModelContext<B>,
                ) -> Result<OutputSchema, String> {
                    // Here we would implement the prediction logic
                    println!("Predict called with input: {:?}", input);
                    Ok(self.model.predict(input, context).map_err(|e| e.to_string())?)
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

                fn into_router(self) -> Router {
                    use axum::routing::post;
                    use axum::routing::get;
                    use axum::routing::Router;
                    use axum::Json;

                    #[derive(Clone)]
                    struct RouterState<B: Backend> {
                        service: Arc<TestService<B>>,
                        context: Arc<ModelContext<B>>,
                    }

                    async fn predict_handler<B: Backend>(
                        State(state): State<RouterState<B>>,
                        Json(params): Json<InputSchema>,
                    ) -> axum::response::Response {
                        // Here we would call the predict method
                        let res = state.service.predict(
                            params,
                            (*state.context).clone(),
                        );
                        match res {
                            Ok(output) => Json(output).into_response(),
                            Err(err) => {
                                axum::response::Response::builder()
                                    .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                                    .body(axum::body::Body::from(err))
                                    .unwrap()
                            }
                        }
                    }

                    let state = RouterState {
                        service: Arc::new(self),
                        context: Arc::new(ModelContext {
                            client: None, // Assuming client is not needed for this example
                            devices: vec![B::Device::default()],
                        }),
                    };
                    let router = Router::new()
                        // for each method in the service annotated with `#[api]` we generate a route
                        .route("/predict", post(predict_handler))
                        .with_state(state);
                    router
                }
            }

            type BackendImpl = burn::backend::NdArray;

            let client = BurnCentralClientConfig::builder(
                BurnCentralCredentials::new("6a18e7be-d027-45f0-a1e9-f0c67693aeb7".to_string()),
                ProjectPath::try_from("jwric/default".to_string()).unwrap(),
            )
                .build();
            let client = tokio::task::spawn_blocking(move || BurnCentralClient::create(client).unwrap()).await.unwrap();

            let devices = vec![NdArrayDevice::default()];

            let app =
                Router::new().route("/", axum::routing::get(|| async { "Hello, World!" }));

            let context: ModelContext<BackendImpl> = ModelContext { client: Some(client), devices };
            // let model_loader = TestModelLoader;
            // let model: TestModel<BackendImpl> = model_loader
            //     .load_model(context)
            //     .expect("Model should be loaded");
            let model = TestModel::<BackendImpl>::init(&NdArrayDevice::default());
            let service = TestService::<BackendImpl>::new(model);
            let service_router = service.into_router();

            let app = app.merge(service_router);

            let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
            axum::serve(listener, app)
                .await
                .expect("Failed to start the service");
        }
    }
}
