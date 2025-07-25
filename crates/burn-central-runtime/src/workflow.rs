mod executor {
    use anyhow::{Context, Result};
    use burn::prelude::{Backend, Module};
    use burn::tensor::backend::AutodiffBackend;
    use burn_central_client::command::MultiDevice;
    use burn_central_client::experiment::{
        ExperimentConfig, ExperimentRun, ExperimentTrackerError, deserialize_and_merge_with_default,
    };
    use burn_central_client::record::ArtifactKind;
    use burn_central_client::{BurnCentral, BurnCentralError};
    use std::any::{Any, TypeId};
    use std::cell::{Ref, RefCell, RefMut};
    use std::collections::HashMap;
    use std::marker::PhantomData;
    use std::ops::{Deref, DerefMut};
    use std::rc::Rc;

    // 3. SystemParam Trait (adapted)
    // How handlers get their arguments from the mutable ExecutionContext
    pub trait SystemParam<B: Backend> {
        type Item<'new>;

        /// This method retrieves the parameter from the context.
        fn retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Self::Item<'r> {
            Self::try_retrieve(ctx).unwrap()
        }

        fn try_retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Result<Self::Item<'r>>;
    }

    pub trait IntoSystem<Input, Output, B: Backend> {
        type System: System<B>;

        // This method converts a function or closure into a System
        fn into_system(self) -> Self::System;
    }

    impl<'ctx, B: Backend> SystemParam<B> for &'ctx ExecutionContext<B> {
        type Item<'new> = &'new ExecutionContext<B>;

        fn try_retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Result<Self::Item<'r>> {
            Ok(ctx)
        }
    }

    pub struct Config<T>(pub T);

    impl<T> Deref for Config<T> {
        type Target = T;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl<'ctx, B: Backend, C: ExperimentConfig> SystemParam<B> for Config<C> {
        type Item<'new> = Config<C>;

        fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
            // Assuming we have a way to get the config from the context
            // For simplicity, let's just return a default config here
            let cfg = match ctx.config_override {
                Some(ref json) => deserialize_and_merge_with_default(json).unwrap_or_default(),
                None => C::default(),
            };
            Ok(Config(cfg))
        }
    }

    impl<B: Backend, M: Module<B> + Default> SystemParam<B> for Model<M> {
        type Item<'new> = Model<M>;

        fn try_retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Result<Self::Item<'r>> {
            // Assuming we have a way to get the model from the context
            // For simplicity, let's just return a default model here
            let model = M::default();
            Ok(Model(model))
        }
    }

    pub struct Res<'a, T: 'static> {
        value: Ref<'a, Box<dyn Any>>,
        _marker: PhantomData<&'a T>,
    }

    impl<T: 'static> Deref for Res<'_, T> {
        type Target = T;

        fn deref(&self) -> &T {
            self.value.downcast_ref().unwrap()
        }
    }

    pub struct ResMut<'a, T: 'static> {
        value: RefMut<'a, Box<dyn Any>>,
        _marker: PhantomData<&'a mut T>,
    }

    impl<T: 'static> Deref for ResMut<'_, T> {
        type Target = T;

        fn deref(&self) -> &T {
            self.value.downcast_ref().unwrap()
        }
    }

    impl<T: 'static> DerefMut for ResMut<'_, T> {
        fn deref_mut(&mut self) -> &mut T {
            self.value.downcast_mut().unwrap()
        }
    }

    impl<'ctx, B: Backend, T: 'static> SystemParam<B> for Res<'ctx, T> {
        type Item<'new> = Res<'new, T>;

        fn try_retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Result<Self::Item<'r>> {
            let value = ctx
                .resources
                .get(&TypeId::of::<T>())
                .context("Resource not found")?
                .borrow();
            Ok(Res {
                value,
                _marker: PhantomData,
            })
        }
    }

    impl<'ctx, B: Backend, T: 'static> SystemParam<B> for ResMut<'ctx, T> {
        type Item<'new> = ResMut<'new, T>;

        fn try_retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Result<Self::Item<'r>> {
            let value = ctx
                .resources
                .get(&TypeId::of::<T>())
                .context("Resource not found")?
                .borrow_mut();
            Ok(ResMut {
                value,
                _marker: PhantomData,
            })
        }
    }

    impl<'ctx, B: Backend> SystemParam<B> for &'ctx ExperimentRun {
        type Item<'new> = &'new ExperimentRun;

        fn try_retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Result<Self::Item<'r>> {
            ctx.experiment
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Experiment run not found"))
                .map(|exp| exp)
        }
    }

    impl<'ctx, B: Backend, P: SystemParam<B>> SystemParam<B> for Option<P> {
        type Item<'new> = Option<P::Item<'new>>;

        fn try_retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Result<Self::Item<'r>> {
            match P::try_retrieve(ctx) {
                Ok(item) => Ok(Some(item)),
                Err(_) => Ok(None), // If retrieval fails, return None
            }
        }
    }

    impl<B: Backend> SystemParam<B> for MultiDevice<B> {
        type Item<'new> = MultiDevice<B>;

        fn try_retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Result<Self::Item<'r>> {
            Ok(MultiDevice(vec![Default::default()]))
        }
    }

    pub trait System<B: Backend>: Send + Sync {
        // Added Send + Sync for potential threading later
        fn run(&self, ctx: &mut ExecutionContext<B>) -> Result<()>;
    }

    /// This trait defines how a specific return type (Output) from a handler
    /// is processed and potentially stored back into the ExecutionContext.
    pub trait IntoSystemOutput<B: Backend>: Send + Sync + 'static {
        /// This method takes the owned output and the mutable ExecutionContext,
        /// allowing the output to modify the context.
        fn apply_output(self: Box<Self>, ctx: &mut ExecutionContext<B>) -> Result<()>;
    }

    impl<B: Backend> IntoSystemOutput<B> for () {
        fn apply_output(self: Box<Self>, _ctx: &mut ExecutionContext<B>) -> Result<()> {
            Ok(()) // Do nothing, successful operation.
        }
    }

    impl<T, E, B: Backend> IntoSystemOutput<B> for core::result::Result<T, E>
    where
        T: IntoSystemOutput<B>,
        E: std::fmt::Display + Send + Sync + 'static,
    {
        fn apply_output(self: Box<Self>, ctx: &mut ExecutionContext<B>) -> Result<()> {
            match *self {
                Ok(output) => Box::new(output).apply_output(ctx),
                Err(e) => Err(anyhow::anyhow!("Error applying output: {}", e)),
            }
        }
    }

    pub struct Model<M>(pub M);

    impl<M> Deref for Model<M> {
        type Target = M;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl<M> From<M> for Model<M> {
        fn from(model: M) -> Self {
            Model(model)
        }
    }

    impl<B: Backend, M: Module<B> + Sync + 'static> IntoSystemOutput<B> for Model<M> {
        fn apply_output(self: Box<Self>, ctx: &mut ExecutionContext<B>) -> Result<()> {
            // Here we could save the model to a file or update the context
            // For simplicity, let's just print a message
            if let Some(experiment) = ctx.experiment.as_ref() {
                experiment.try_log_artifact("model", ArtifactKind::Model, self.0.into_record())?;
            } else {
                println!("No experiment run to log the model.");
            }
            Ok(())
        }
    }

    pub struct TrainingStep<F>(pub F);

    pub struct ErasedSystem<B: Backend> {
        func: Box<
            dyn Fn(&mut ExecutionContext<B>) -> Result<Box<dyn IntoSystemOutput<B>>> + Send + Sync,
        >,
    }

    // pub trait TrainingOutput<B: Backend>: Send + Sync + 'static {
    //     fn apply_training(self: Box<Self>, ctx: &mut ExecutionContext<B>) -> Result<()>;
    // }
    //
    // impl<M, B> TrainingOutput<B> for Result<M>
    // where
    //     M: Module<B> + Send + Sync + 'static,
    //     B: Backend,
    // {
    //     fn apply_training(self: Box<Self>, ctx: &mut ExecutionContext<B>) -> Result<()> {
    //         println!("Saving model in TrainingStep: {:?}", self);
    //
    //         Ok(())
    //     }
    // }

    impl<B: Backend> System<B> for ErasedSystem<B> {
        fn run(&self, ctx: &mut ExecutionContext<B>) -> Result<()> {
            let output = (self.func)(ctx)?;
            output.apply_output(ctx)
        }
    }

    macro_rules! impl_into_system {
        ($($P:ident),*) => {
            impl<Func, R: 'static, B, $($P: SystemParam<B>),*> IntoSystem<($($P,)*), R, B> for Func
            where
                B: Backend,
                for<'a, 'b> &'a Func:
                    Fn($($P),*) -> R +
                    Fn($(<$P as SystemParam<B>>::Item<'b>),*) -> R,
                Func: Fn($($P),*) -> R + Send + Sync + 'static,
                R: IntoSystemOutput<B>,
            {
                type System = ErasedSystem<B>;

                fn into_system(self) -> Self::System {
                    fn call_inner<R, $($P),*>(
                        f: impl Fn($($P),*) -> R,
                        $($P: $P),*
                    ) -> R
                    {
                        f($($P),*)
                    }

                    ErasedSystem {
                        func: Box::new(move |ctx| {
                            // retrieve params
                            $(let $P = <$P as SystemParam<B>>::try_retrieve(ctx).context("Failed to retrieve parameter")?;)*
                            let output = call_inner(&self, $($P),*);
                            Ok(Box::new(output) as Box<dyn IntoSystemOutput<B>>)
                        })
                    }
                }
            }

        //     impl<Func, R: 'static, B, $($P: SystemParam<B>),*> IntoSystem<($($P,)*), R, B> for TrainingStep<Func>
        //     where
        //         B: Backend,
        //         for<'a, 'b> &'a Func:
        //             Fn($($P),*) -> R +
        //             Fn($(<$P as SystemParam<B>>::Item<'b>),*) -> R,
        //         Func: Fn($($P),*) -> R + Send + Sync + 'static,
        //         R: TrainingOutput<B>,
        //     {
        //         type System = ErasedSystem<B>;
        //
        //         fn into_system(self) -> Self::System {
        //             fn call_inner<R, $($P),*>(
        //                 f: impl Fn($($P),*) -> R,
        //                 $($P: $P),*
        //             ) -> R
        //             {
        //                 f($($P),*)
        //             }
        //
        //             ErasedSystem {
        //                 func: Box::new(move |ctx| {
        //                     // retrieve params
        //                     $(let $P = <$P as SystemParam<B>>::retrieve(ctx);)*
        //                     let training_output = call_inner(&self.0, $($P),*);
        //                     let output = Box::new(training_output).apply_training(ctx);
        //                     Ok(Box::new(output) as Box<dyn IntoSystemOutput<B>>)
        //                 })
        //             }
        //         }
        //     }
        };
    }

    impl_into_system!();
    impl_into_system!(P1);
    impl_into_system!(P1, P2);
    impl_into_system!(P1, P2, P3);
    impl_into_system!(P1, P2, P3, P4);
    impl_into_system!(P1, P2, P3, P4, P5);
    impl_into_system!(P1, P2, P3, P4, P5, P6);

    pub struct DirectSystem<S>(pub S);

    impl<B: Backend, I, O, S> IntoSystem<I, O, B> for DirectSystem<S>
    where
        S: System<B>,
    {
        type System = S;

        fn into_system(self) -> Self::System {
            self.0
        }
    }

    // --- Custom Error Type ---
    #[derive(thiserror::Error, Debug)]
    pub enum RuntimeError {
        #[error("Handler '{0}' not found")]
        HandlerNotFound(String),
        #[error("Resource of type {0} not found")]
        ResourceNotFound(String),
        #[error("Resource is already borrowed mutably")]
        ResourceBorrowFailed,
        #[error("Burn Central API call failed: {0}")]
        BurnCentralError(#[from] BurnCentralError),
        #[error("Experiment API call failed: {0}")]
        ExperimentApiFailed(#[from] ExperimentTrackerError),
        #[error("Handler execution failed: {0}")]
        HandlerFailed(anyhow::Error),
    }

    pub struct ExecutionContext<B: Backend> {
        client: Option<BurnCentral>,
        namespace: String,
        project: String,
        config_override: Option<String>,
        devices: Vec<B::Device>,
        experiment: Option<ExperimentRun>,
        resources: Rc<HashMap<TypeId, RefCell<Box<dyn Any>>>>,
    }

    impl<B: Backend> ExecutionContext<B> {
        pub fn get_merged_config<C: ExperimentConfig>(&self) -> C {
            match &self.config_override {
                Some(json) => deserialize_and_merge_with_default(json).unwrap_or_default(),
                None => C::default(),
            }
        }

        // Users of the context (handlers) will use these methods
        pub fn experiment(&self) -> Option<&ExperimentRun> {
            self.experiment.as_ref()
        }

        pub fn devices(&self) -> &[B::Device] {
            &self.devices
        }

        pub fn resource<T: Any + 'static>(&self) -> Result<Ref<T>, RuntimeError> {
            self.resources
                .get(&TypeId::of::<T>())
                .ok_or_else(|| {
                    RuntimeError::ResourceNotFound(std::any::type_name::<T>().to_string())
                })?
                .try_borrow()
                .map(|r| Ref::map(r, |b| b.downcast_ref::<T>().unwrap()))
                .map_err(|_| RuntimeError::ResourceBorrowFailed)
        }

        pub fn resource_mut<T: Any + 'static>(&self) -> Result<RefMut<T>, RuntimeError> {
            self.resources
                .get(&TypeId::of::<T>())
                .ok_or_else(|| {
                    RuntimeError::ResourceNotFound(std::any::type_name::<T>().to_string())
                })?
                .try_borrow_mut()
                .map(|r| RefMut::map(r, |b| b.downcast_mut::<T>().unwrap()))
                .map_err(|_| RuntimeError::ResourceBorrowFailed)
        }
    }

    pub struct ExecutorBuilder<B: Backend> {
        executor: Executor<B>,
    }

    impl<B: Backend> ExecutorBuilder<B> {
        fn new(client: BurnCentral, namespace: String, project: String) -> Self {
            Self {
                executor: Executor {
                    client,
                    namespace,
                    project,
                    handlers: HashMap::new(),
                    resources: Rc::new(HashMap::new()),
                },
            }
        }

        pub fn add_handler<I, O, S: System<B> + 'static>(
            &mut self,
            name: &str,
            handler: impl IntoSystem<I, O, B, System = S>,
        ) -> &mut Self {
            self.executor
                .handlers
                .insert(name.to_string(), Box::new(handler.into_system()));
            self
        }

        pub fn add_resource<T: Any + 'static>(&mut self, resource: T) -> &mut Self {
            Rc::get_mut(&mut self.executor.resources)
                .unwrap()
                .insert(TypeId::of::<T>(), RefCell::new(Box::new(resource)));
            self
        }

        pub fn init_resource<T: Any + Default + 'static>(&mut self) -> &mut Self {
            let type_id = TypeId::of::<T>();
            if !self.executor.resources.contains_key(&type_id) {
                Rc::get_mut(&mut self.executor.resources)
                    .unwrap()
                    .insert(type_id, RefCell::new(Box::new(T::default())));
            }
            self
        }

        pub fn build(self) -> Executor<B> {
            self.executor
        }
    }

    pub struct Executor<B: Backend> {
        client: BurnCentral,
        namespace: String,
        project: String,
        handlers: HashMap<String, Box<dyn System<B>>>,
        resources: Rc<HashMap<TypeId, RefCell<Box<dyn Any>>>>,
    }

    impl<B: Backend> Executor<B> {
        // The main entry point is now a builder
        pub fn builder(
            client: BurnCentral,
            namespace: impl Into<String>,
            project: impl Into<String>,
        ) -> ExecutorBuilder<B> {
            ExecutorBuilder::new(client, namespace.into(), project.into())
        }

        pub fn targets(&self) -> Vec<String> {
            self.handlers.keys().cloned().collect()
        }

        // This runs a single chain of handlers with an initial context
        pub fn run(
            &self,
            target: &str,
            devices: Vec<B::Device>,
            config_override: Option<String>,
        ) -> Result<(), RuntimeError> {
            println!("--- Starting Execution for Target: {} ---", target);

            let handler = self
                .handlers
                .get(target)
                .ok_or_else(|| RuntimeError::HandlerNotFound(target.to_string()))?;

            let mut ctx = ExecutionContext {
                client: Some(self.client.clone()),
                namespace: self.namespace.clone(),
                project: self.project.clone(),
                config_override,
                devices,
                experiment: None,
                resources: self.resources.clone(),
            };

            let config = ctx.config_override.as_deref().unwrap_or("{}");

            if let Some(client) = &mut ctx.client {
                let experiment = client.start_experiment(&ctx.namespace, &ctx.project, &config)?;
                ctx.experiment = Some(experiment);
            }

            let result = handler.run(&mut ctx);

            match result {
                Ok(_) => {
                    if let Some(experiment) = ctx.experiment {
                        experiment.finish()?;
                        println!("Experiment run completed successfully.");
                    }
                    println!("Handler {} executed successfully.", target);

                    Ok(())
                }
                Err(e) => {
                    println!("Error executing handler '{}': {}", target, e);
                    // Handle the error, possibly logging it or cleaning up
                    if let Some(experiment) = ctx.experiment {
                        experiment.fail(e.to_string())?;
                        println!("Experiment run failed: {}", e);
                    }
                    Err(RuntimeError::HandlerFailed(e))
                }
            }
        }
    }

    // --- Example Handlers ---

    #[cfg(test)]
    mod test {
        use super::*;
        use burn_central_client::credentials::BurnCentralCredentials;
        use serde::{Deserialize, Serialize};

        #[derive(Module, Debug)]
        pub struct TestModel<B: Backend> {
            // Define your model parameters here
            _backend: PhantomData<B>,
        }

        impl<B: Backend> Default for TestModel<B> {
            fn default() -> Self {
                TestModel {
                    _backend: PhantomData,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct SomeExperimentConfig {
            pub param1: f32,
            pub param2: String,
        }

        impl Default for SomeExperimentConfig {
            fn default() -> Self {
                SomeExperimentConfig {
                    param1: 0.0,
                    param2: "default".to_string(),
                }
            }
        }

        fn log_model2<B: Backend>(
            experiment: &ExperimentRun,
            Model(_a): Model<TestModel<B>>,
            Config(config): Config<SomeExperimentConfig>,
            context: &ExecutionContext<B>,
        ) -> Result<Model<TestModel<B>>, RuntimeError> {
            println!("  Logging model...");

            experiment.log_info(format!("Logging model with config: {:?}", config));

            Ok(_a.into())
        }

        // Handler that modifies experiment_data
        // fn preprocess_data<B: Backend>(config: Config<SomeExperimentConfig>) -> Result<()> {
        //     println!("  Preprocessed data. New data: {}", data);
        //     Ok(())
        // }

        // Handler that reads data and writes a model path
        fn train_model<B: Backend>(
            config: Config<SomeExperimentConfig>,
            model: Model<TestModel<B>>,
        ) {
            println!("  Training model with data: {:?}", *config);
            // Simulate some training logic
            // *model_path = Some(format!("/models/{}-v1.pkl", data));

            println!("  Model trained. Path: {:?}", config.param1);
        }

        // // Handler that uses the model path to evaluate
        // fn evaluate_model(model_path: &Option<String>, results: &Vec<f32>) -> Result<()> {
        //     if let Some(path) = model_path {
        //         println!("  Evaluating model from path: {}", path);
        //         // results.push(0.95); // Add a metric
        //     } else {
        //         println!("  No model path to evaluate.");
        //     }
        //     println!("  Results after evaluation: {:?}", results);
        //     Ok(())
        // }

        // Handler that takes no arguments
        fn log_completion() -> Result<()> {
            println!("  Experiment run completed!");
            Ok(())
        }

        type Back = burn::backend::NdArray;
        type Device = <Back as Backend>::Device;

        #[test]
        fn test_executor_api() {
            // Add handlers to the executor
            // executor.add_handler(preprocess_data::<Back>);

            // executor.add_handler(evaluate_model);
            // executor.add_handler(TrainingStep(log_model::<Back>));

            let client = BurnCentral::builder(BurnCentralCredentials::new(
                "8543d2e1-1b48-4205-9d5e-3cd282126ec1",
            ))
            .with_endpoint("http://localhost:9001")
            .build()
            .expect("Failed to build BurnCentral client");

            let override_json = serde_json::to_string(&SomeExperimentConfig {
                param1: 42.0,
                param2: "example".to_string(),
            })
            .expect("Failed to serialize config");

            // Create an initial context for a specific experiment run
            let mut builder = Executor::<Back>::builder(client, "aaa", "aaaa");

            build_executor(&mut builder);

            let executor = builder.build();

            executor
                .run("log_model2", vec![Default::default()], Some(override_json))
                .expect("Execution failed");
        }

        // This would be the function that the user implements to build the executor in their application
        fn build_executor<B: Backend>(exec: &mut ExecutorBuilder<B>) {
            exec.add_handler("train_model", train_model)
                .add_handler("log_model2", log_model2)
                .init_resource::<TestModel<Back>>();
        }
    }
}
