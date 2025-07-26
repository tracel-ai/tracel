#![allow(unused)]
mod executor {
    use crate::type_name::fn_type_name;
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
    use variadics_please::all_tuples;

    // 3. SystemParam Trait (adapted)
    // How handlers get their arguments from the mutable ExecutionContext
    pub trait SystemParam<B: Backend>: Sized {
        type Item<'new>;

        /// This method retrieves the parameter from the context.
        fn retrieve(ctx: &ExecutionContext<B>) -> Self::Item<'_> {
            Self::try_retrieve(ctx).unwrap()
        }

        fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>>;
    }

    impl<'ctx, B: Backend> SystemParam<B> for &'ctx ExecutionContext<B> {
        type Item<'new> = &'new ExecutionContext<B>;

        fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
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
            let cfg = ctx.get_merged_config();
            Ok(Config(cfg))
        }
    }

    impl<B: Backend, M: Module<B> + Default> SystemParam<B> for Model<M> {
        type Item<'new> = Model<M>;

        fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
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

        fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
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

        fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
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

        fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
            ctx.experiment
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Experiment run not found"))
                .map(|exp| exp)
        }
    }

    impl<'ctx, B: Backend, P: SystemParam<B>> SystemParam<B> for Option<P> {
        type Item<'new> = Option<P::Item<'new>>;

        fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
            match P::try_retrieve(ctx) {
                Ok(item) => Ok(Some(item)),
                Err(_) => Ok(None), // If retrieval fails, return None
            }
        }
    }

    impl<B: Backend> SystemParam<B> for MultiDevice<B> {
        type Item<'new> = MultiDevice<B>;

        fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
            Ok(MultiDevice(ctx.devices.clone()))
        }
    }

    // for all tuples
    macro_rules! impl_system_param_tuple {
        ($($P:ident),*) => {
            #[expect(
                clippy::allow_attributes,
                reason = "This is in a macro, and as such, the below lints may not always apply."
            )]
            #[allow(
                non_snake_case,
                reason = "Certain variable names are provided by the caller, not by us."
            )]
            #[allow(
                unused_variables,
                reason = "Zero-length tuples won't use some of the parameters."
            )]
            impl<B: Backend, $($P: SystemParam<B>),*> SystemParam<B> for ($($P,)*) {
                type Item<'new> = ($($P::Item<'new>,)*);

                fn try_retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Result<Self::Item<'r>> {
                    Ok((
                        $(<$P as SystemParam<B>>::try_retrieve(ctx)?,)*
                    ))
                }
            }
        };
    }

    all_tuples!(impl_system_param_tuple, 0, 16, P);

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

    // pub struct TrainingStep<F>(pub F);
    #[diagnostic::on_unimplemented(message = "`{Self}` is not a system", label = "invalid system")]
    pub trait System<B: Backend>: Send + Sync + 'static {
        type Out;

        fn name(&self) -> &str;
        fn run(&self, ctx: &mut ExecutionContext<B>) -> Result<Self::Out, RuntimeError>;
    }
    pub type BoxedSystem<B, Out = ()> = Box<dyn System<B, Out = Out>>;
    pub type ExecutorSystem<B> = BoxedSystem<B, ()>;

    pub type SystemParamItem<'ctx, B, P> = <P as SystemParam<B>>::Item<'ctx>;

    #[diagnostic::on_unimplemented(
        message = "`{Self}` is not a valid system",
        label = "invalid system"
    )]
    pub trait SystemParamFunction<B: Backend, Marker>: Send + Sync + 'static {
        type Out: IntoSystemOutput<B> + 'static;
        type Param: SystemParam<B>;

        fn run(
            &self,
            param_value: SystemParamItem<B, Self::Param>,
        ) -> Result<Self::Out, RuntimeError>;
    }

    macro_rules! impl_system_function {
        ($($param: ident),*) => {
            #[expect(
                clippy::allow_attributes,
                reason = "This is within a macro, and as such, the below lints may not always apply."
            )]
            #[allow(
                non_snake_case,
                reason = "Certain variable names are provided by the caller, not by us."
            )]
            impl<B: Backend, Out, Func, $($param: SystemParam<B>),*> SystemParamFunction<B, fn($($param,)*) -> Out> for Func
            where
                Func: Send + Sync + 'static,
                for <'a> &'a Func:
                    Fn($($param),*) -> Out +
                    Fn($(SystemParamItem<B, $param>),*) -> Out,
                Out: IntoSystemOutput<B> + 'static,
            {
                type Out = Out;
                type Param = ($($param,)*);
                #[inline]
                fn run(&self, param_value: SystemParamItem<B, ($($param,)*)>) -> Result<Self::Out, RuntimeError> {
                    fn call_inner<Out, $($param,)*>(
                        f: impl Fn($($param,)*)->Out,
                        $($param: $param,)*
                    )->Out{
                        f($($param,)*)
                    }
                    let ($($param,)*) = param_value;
                    Ok(call_inner(self, $($param),*))
                }
            }
        };
    }

    all_tuples!(impl_system_function, 0, 16, F);

    #[doc(hidden)]
    pub struct IsFunctionSystem;

    pub struct FunctionSystem<B, Marker, F>
    where
        F: SystemParamFunction<B, Marker>,
        B: Backend,
    {
        func: F,
        name: String,
        _marker: PhantomData<fn() -> (B, Marker)>,
    }

    impl<B, Marker, F> FunctionSystem<B, Marker, F>
    where
        F: SystemParamFunction<B, Marker>,
        B: Backend,
    {
        pub fn with_name(mut self, name: impl Into<String>) -> Self {
            self.name = name.into();
            self
        }
    }

    impl<B, Marker, F> Clone for FunctionSystem<B, Marker, F>
    where
        F: SystemParamFunction<B, Marker> + Clone,
        B: Backend,
    {
        fn clone(&self) -> Self {
            FunctionSystem {
                func: self.func.clone(),
                name: self.name.clone(),
                _marker: PhantomData,
            }
        }
    }

    impl<B, Marker, F> IntoSystem<B, (), (IsFunctionSystem, Marker)> for F
    where
        B: Backend,
        Marker: 'static,
        F: SystemParamFunction<B, Marker>,
    {
        type System = FunctionSystem<B, Marker, F>;

        fn into_system(func: Self) -> Self::System {
            FunctionSystem {
                func,
                name: fn_type_name::<F>(),
                _marker: PhantomData,
            }
        }
    }

    impl<B, Marker, F> System<B> for FunctionSystem<B, Marker, F>
    where
        B: Backend,
        Marker: 'static,
        F: SystemParamFunction<B, Marker>,
    {
        type Out = ();

        fn name(&self) -> &str {
            self.name.as_str()
        }

        fn run(&self, ctx: &mut ExecutionContext<B>) -> Result<Self::Out, RuntimeError> {
            let params = F::Param::try_retrieve(ctx).map_err(|e| {
                RuntimeError::HandlerFailed(anyhow::anyhow!("Failed to retrieve parameters: {}", e))
            })?;
            let output = self.func.run(params)?;
            Box::new(output).apply_output(ctx).map_err(|e| {
                RuntimeError::HandlerFailed(anyhow::anyhow!("Failed to apply output: {}", e))
            })
        }
    }

    impl<B: Backend, T: System<B>> IntoSystem<B, T::Out, ()> for T {
        type System = T;
        fn into_system(this: Self) -> Self::System {
            this
        }
    }

    #[diagnostic::on_unimplemented(
        message = "`{Self}` is not a valid system with output `{Output}`",
        label = "invalid system"
    )]
    pub trait IntoSystem<B: Backend, Output, Marker>: Sized {
        type System: System<B, Out = Output>;

        #[allow(clippy::wrong_self_convention)]
        fn into_system(this: Self) -> Self::System;
    }

    // --- System modifiers ---

    /// An extension trait for `IntoSystem` that provides system-modifying methods.
    pub trait IntoSystemExt<B: Backend, O, M>: IntoSystem<B, O, M> {
        /// Assigns a custom name to a system, overriding the default.
        ///
        /// The default name for a function system is derived from its type, which is unique.
        /// This modifier allows you to register the same system function multiple times
        /// under different names, which can be useful for creating distinct stages in a
        /// workflow that use the same logic.
        ///
        /// # Example
        ///
        /// ```ignore
        /// fn my_system_logic() {}
        ///
        /// let mut builder = Executor::builder(...);
        /// builder.add_handler(my_system_logic.with_name("stage_one"));
        /// builder.add_handler(my_system_logic.with_name("stage_two"));
        ///
        /// let executor = builder.build();
        /// executor.run("stage_one", ...)?;
        /// ```
        fn with_name(self, name: impl Into<String>) -> Named<Self> {
            Named {
                system: self,
                name: name.into(),
            }
        }
    }

    // Blanket implementation for any type that can be turned into a system.
    impl<B: Backend, O, M, T> IntoSystemExt<B, O, M> for T where T: IntoSystem<B, O, M> {}

    /// A wrapper for an `IntoSystem`-implementing type that holds a custom name.
    /// This is constructed by the `.with_name()` method from the `IntoSystemExt` trait.
    pub struct Named<S> {
        system: S,
        name: String,
    }

    impl<S: Clone> Clone for Named<S> {
        fn clone(&self) -> Self {
            Self {
                system: self.system.clone(),
                name: self.name.clone(),
            }
        }
    }

    /// A `System` that wraps another `System` to override its name.
    /// This is the final system type that the executor interacts with.
    pub struct NamedSystem<S, B> {
        inner: S,
        name: String,
        _marker: PhantomData<B>,
    }

    impl<S, B> System<B> for NamedSystem<S, B>
    where
        S: System<B>,
        B: Backend,
    {
        type Out = S::Out;

        fn name(&self) -> &str {
            &self.name
        }

        fn run(&self, ctx: &mut ExecutionContext<B>) -> Result<Self::Out, RuntimeError> {
            // Delegate the `run` call to the inner system.
            self.inner.run(ctx)
        }
    }

    pub struct IsNamedSystem;
    // Implements `IntoSystem` for the `Named` wrapper. This allows a named system to be
    // passed to methods like `add_handler`.
    impl<B, O, M, S> IntoSystem<B, O, (IsNamedSystem, M)> for Named<S>
    where
        B: Backend,
        S: IntoSystem<B, O, M, System: System<B, Out = O>>,
    {
        type System = NamedSystem<S::System, B>;

        fn into_system(this: Self) -> Self::System {
            NamedSystem {
                inner: IntoSystem::into_system(this.system),
                name: this.name,
                _marker: PhantomData,
            }
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

        fn get_res_cell<T: Any + 'static>(&self) -> Result<&RefCell<Box<dyn Any>>, RuntimeError> {
            self.resources
                .get(&TypeId::of::<T>())
                .ok_or_else(|| {
                    RuntimeError::ResourceNotFound(std::any::type_name::<T>().to_string())
                })
        }

        pub fn resource<T: Any + 'static>(&self) -> Result<Ref<T>, RuntimeError> {
            self.get_res_cell::<T>()?
                .try_borrow()
                .map(|r| Ref::map(r, |b| b.downcast_ref::<T>().unwrap()))
                .map_err(|_| RuntimeError::ResourceBorrowFailed)
        }

        pub fn resource_mut<T: Any + 'static>(&self) -> Result<RefMut<T>, RuntimeError> {
            self.get_res_cell::<T>()?
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

        pub fn add_handler<M>(&mut self, handler: impl IntoSystem<B, (), M>) -> &mut Self
        {
            let system = Box::new(IntoSystem::into_system(handler));
            let name = system.name();
            println!("Adding handler: {}", name);
            self.executor.handlers.insert(name.to_string(), system);
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
        handlers: HashMap<String, ExecutorSystem<B>>,
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
            target: impl AsRef<str>,
            devices: impl IntoIterator<Item = B::Device>,
            config_override: Option<String>,
        ) -> Result<(), RuntimeError> {
            println!("--- Starting Execution for Target: {} ---", target.as_ref());

            let handler = self
                .handlers
                .get(target.as_ref())
                .ok_or_else(|| RuntimeError::HandlerNotFound(target.as_ref().to_string()))?;

            let mut ctx = ExecutionContext {
                client: Some(self.client.clone()),
                namespace: self.namespace.clone(),
                project: self.project.clone(),
                config_override,
                devices: devices.into_iter().collect(),
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
                    println!("Handler {} executed successfully.", target.as_ref());

                    Ok(())
                }
                Err(e) => {
                    println!("Error executing handler '{}': {}", target.as_ref(), e);
                    // Handle the error, possibly logging it or cleaning up
                    if let Some(experiment) = ctx.experiment {
                        experiment.fail(e.to_string())?;
                        println!("Experiment run failed: {}", e);
                    }
                    Err(e)
                }
            }
        }
    }

    // --- Example Handlers ---

    #[cfg(test)]
    mod test {
        use burn::backend::{Autodiff, NdArray};
        use burn::prelude::Backend;
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

        mod derive_api {
            use burn::prelude::Backend;
            use crate::workflow::executor::{ExecutionContext, IntoSystem};
            use serde::{Deserialize, Serialize};

            // #[derive(Experiment)]
            #[derive(Serialize, Deserialize, Debug)]
            pub struct DerivedExperimentConfig {
                pub param1: f32,
                pub param2: String,
            }

            impl Default for DerivedExperimentConfig {
                fn default() -> Self {
                    DerivedExperimentConfig {
                        param1: 0.0,
                        param2: "default".to_string(),
                    }
                }
            }

            // #[experiment_impl]
            impl DerivedExperimentConfig {
                // #[experiment(name = "test_associated_system")]
                pub fn test_associated_system<B: Backend>(
                    self,
                    ctx: &ExecutionContext<B>,
                ) -> anyhow::Result<()> {
                    // Example of using the context to log something
                    if let Some(experiment) = ctx.experiment() {
                        experiment.log_info(format!(
                            "Running test_associated_system with param1: {}",
                            self.param1
                        ));
                    }
                    Ok(())
                }
            }
            // generated code by the #[experiment_impl] macro
            // ...
        }

        // #[derive(Experiment)]
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

        fn log_model2<B: AutodiffBackend>(
            experiment: &ExperimentRun,
            Config(config): Config<SomeExperimentConfig>,
            context: &ExecutionContext<B>,
        ) -> Result<Model<TestModel<B>>> {
            println!("  Logging model...");

            experiment.log_info(format!("Logging model with config: {:?}", config));

            // Ok(_a.into())
            anyhow::bail!("Not implemented")
        }

        fn test_model_validation<B: Backend>(
            config: Config<SomeExperimentConfig>,
            _model: Model<TestModel<B>>,
            _context: &ExecutionContext<B>,
        ) -> Result<(), RuntimeError> {
            println!("  Validating config: {:?}", *config);
            if config.param1 < 0.0 {
                return Err(RuntimeError::HandlerFailed(anyhow::anyhow!(
                    "param1 must be non-negative"
                )));
            }
            Ok(())
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

        type Back = Autodiff<NdArray>;

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


        pub struct CustomSystemStruct<B: AutodiffBackend> {
            _marker: PhantomData<B>,
        }

        impl<B: AutodiffBackend> CustomSystemStruct<B> {
            pub fn new() -> Self {
                CustomSystemStruct {
                    _marker: PhantomData,
                }
            }
        }

        impl<B: AutodiffBackend> System<B> for CustomSystemStruct<B> {
            type Out = ();

            fn name(&self) -> &str {
                "CustomSystemStruct"
            }

            fn run(&self, ctx: &mut ExecutionContext<B>) -> Result<Self::Out, RuntimeError> {
                // Example logic for the system
                println!("Running CustomSystemStruct with context: {:?}", ctx.project);
                Ok(())
            }
        }

        // This would be the function that the user implements to build the executor in their application
        fn build_executor<B: AutodiffBackend>(exec: &mut ExecutorBuilder<B>) {
            exec.add_handler(train_model.with_name("a"))
                .add_handler(log_model2)
                .add_handler(test_model_validation)
                .add_handler(CustomSystemStruct::new().with_name("CustomSystem"))
                .init_resource::<TestModel<Back>>();
        }
    }
}
