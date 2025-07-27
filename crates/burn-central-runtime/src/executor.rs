use crate::type_name::fn_type_name;
use anyhow::{Context, Result};
use burn::prelude::{Backend, Module};

use crate::backend::{AutodiffBackendStub, BackendStub};
use burn::backend::Autodiff;
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

pub trait SystemParam<B: Backend>: Sized {
    type Item<'new>;

    /// This method retrieves the parameter from the context.
    fn retrieve(ctx: &ExecutionContext<B>) -> Self::Item<'_> {
        Self::try_retrieve(ctx).unwrap()
    }

    /// This method attempts to retrieve the parameter from the context, returning an error if it fails.
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
        let cfg = ctx.get_merged_config();
        Ok(Config(cfg))
    }
}

impl<B: Backend, M: Module<B> + Default> SystemParam<B> for Model<M> {
    type Item<'new> = Model<M>;

    fn try_retrieve(_ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
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

    fn run(&self, param_value: SystemParamItem<B, Self::Param>) -> Result<Self::Out, RuntimeError>;
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

pub struct FunctionSystem<Marker, F> {
    func: F,
    name: String,
    _marker: PhantomData<fn() -> (Marker)>,
}

impl<Marker, F> FunctionSystem<Marker, F> {
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

impl<Marker, F: Clone> Clone for FunctionSystem<Marker, F> {
    fn clone(&self) -> Self {
        FunctionSystem {
            func: self.func.clone(),
            name: self.name.clone(),
            _marker: PhantomData,
        }
    }
}

impl<B, Marker, F> IntoSystem<B, (), (IsFunctionSystem, B, Marker)> for F
where
    B: Backend,
    Marker: 'static,
    F: SystemParamFunction<B, Marker>,
{
    type System = FunctionSystem<Marker, F>;

    fn into_system(func: Self) -> Self::System {
        FunctionSystem {
            func,
            name: fn_type_name::<F>(),
            _marker: PhantomData,
        }
    }
}

impl<B, Marker, F> System<B> for FunctionSystem<Marker, F>
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
    fn with_name(self, name: impl Into<String>) -> IntoNamedSystem<Self> {
        IntoNamedSystem {
            system: self,
            name: name.into(),
        }
    }
}

// --- System modifiers ---

/// A wrapper for an `IntoSystem`-implementing type that holds a custom name.
/// This is constructed by the `.with_name()` method from the `IntoSystemExt` trait.
#[derive(Clone)]
pub struct IntoNamedSystem<S> {
    system: S,
    name: String,
}

/// A `System` that wraps another `System` to override its name.
/// This is the final system type that the executor interacts with.
pub struct NamedSystem<S> {
    inner: S,
    name: String,
}

impl<S, B> System<B> for NamedSystem<S>
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

#[doc(hidden)]
pub struct IsNamedSystem;
// Implements `IntoSystem` for the `Named` wrapper. This allows a named system to be
// passed to methods like `add_handler`.
impl<B, O, M, S> IntoSystem<B, O, (IsNamedSystem, B, O, M)> for IntoNamedSystem<S>
where
    B: Backend,
    S: IntoSystem<B, O, M>,
{
    type System = NamedSystem<S::System>;

    fn into_system(this: Self) -> Self::System {
        NamedSystem {
            inner: IntoSystem::into_system(this.system),
            name: this.name,
        }
    }
}

impl<B, O, M, S, N> IntoSystem<B, O, (IsNamedSystem, B, O, M, N)> for (N, S)
where
    B: Backend,
    S: IntoSystem<B, O, M>,
    N: Into<String>,
{
    type System = NamedSystem<S::System>;

    fn into_system(this: Self) -> Self::System {
        let (name, system) = this;
        NamedSystem {
            inner: IntoSystem::into_system(system),
            name: name.into(),
        }
    }
}

#[macro_export]
macro_rules! sys {
    ($system:expr) => {
        (stringify!($system), $system)
    };
}

#[macro_export]
macro_rules! register_handlers {
    ($builder:expr) => {};

    ($builder:expr, $handler:expr => $name:expr, $($rest:tt)*) => {
        $builder.add_handler(($name, $handler));
        register_handlers!($builder, $($rest)*);
    };

    ($builder:expr, $handler:expr => $name:expr) => {
        $builder.add_handler(($name, $handler));
    };

    ($builder:expr, $handler:expr, $($rest:tt)*) => {
        $builder.add_handler((stringify!($handler), $handler));
        register_handlers!($builder, $($rest)*);
    };

    ($builder:expr, $handler:expr) => {
        $builder.add_handler((stringify!($handler), $handler));
    };
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
    #[error("Ambiguous target '{0}'. Found multiple handlers: {1:?}")]
    AmbiguousHandlerName(String, Vec<String>),
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

    pub fn experiment(&self) -> Option<&ExperimentRun> {
        self.experiment.as_ref()
    }

    pub fn devices(&self) -> &[B::Device] {
        &self.devices
    }

    fn get_res_cell<T: Any + 'static>(&self) -> Result<&RefCell<Box<dyn Any>>, RuntimeError> {
        self.resources
            .get(&TypeId::of::<T>())
            .ok_or_else(|| RuntimeError::ResourceNotFound(std::any::type_name::<T>().to_string()))
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

pub trait Plugin<B: Backend> {
    fn build(&self, builder: &mut ExecutorBuilder<B>);
}

pub trait StaticPlugin<B: Backend> {
    fn build(builder: &mut ExecutorBuilder<B>);
}

pub struct ExecutorBuilder<B: Backend> {
    executor: Executor<B>,
    scope_stack: Vec<String>,
}

impl<B: Backend> ExecutorBuilder<B> {
    fn new() -> Self {
        Self {
            executor: Executor {
                client: None,
                namespace: None,
                project: None,
                handlers: HashMap::new(),
                resources: Rc::new(HashMap::new()),
                handler_tags: HashMap::new(),
            },
            scope_stack: Vec::new(),
        }
    }

    pub fn add_handler<M>(&mut self, handler: impl IntoSystem<B, (), M>) -> &mut Self {
        let system = Box::new(IntoSystem::into_system(handler));
        let name = system.name().to_string();

        let full_name = if self.scope_stack.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", self.scope_stack.join("/"), &name)
        };

        println!(
            "Adding handler: {} (base name tag: '{}')",
            &full_name, &name
        );
        self.executor
            .handler_tags
            .entry(name)
            .or_default()
            .push(full_name.clone());

        self.executor.handlers.insert(full_name, system);
        self
    }

    pub fn add_handler_if<M>(
        &mut self,
        handler: impl IntoSystem<B, (), M>,
        condition: bool,
    ) -> &mut Self {
        if condition {
            self.add_handler(handler);
        } else {
            println!("Skipping handler: {}", fn_type_name::<M>());
        }
        self
    }

    pub fn add_plugin(&mut self, plugin: impl Plugin<B>) -> &mut Self {
        plugin.build(self);
        self
    }

    pub fn add_static_plugin<P: StaticPlugin<B>>(&mut self) -> &mut Self {
        P::build(self);
        self
    }

    pub fn scope<F>(&mut self, prefix: &str, add_scoped_handlers: F) -> &mut Self
    where
        F: FnOnce(&mut Self),
    {
        self.scope_stack.push(prefix.to_string());
        add_scoped_handlers(self);
        self.scope_stack.pop();
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

    pub fn build(
        self,
        client: BurnCentral,
        namespace: impl Into<String>,
        project: impl Into<String>,
    ) -> Executor<B> {
        let mut executor = self.executor;
        executor.client = Some(client);
        executor.namespace = Some(namespace.into());
        executor.project = Some(project.into());
        // Possibly do some validation or final setup here
        executor
    }

    fn build_stub(self) -> Executor<B> {
        self.executor
    }
}

pub struct Executor<B: Backend> {
    client: Option<BurnCentral>,
    namespace: Option<String>,
    project: Option<String>,
    handlers: HashMap<String, ExecutorSystem<B>>,
    resources: Rc<HashMap<TypeId, RefCell<Box<dyn Any>>>>,
    handler_tags: HashMap<String, Vec<String>>,
}

impl<B: Backend> Executor<B> {
    // The main entry point is now a builder
    pub fn builder() -> ExecutorBuilder<B> {
        ExecutorBuilder::new()
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
        let target = target.as_ref();

        println!("--- Starting Execution for Target: {} ---", target);

        // 1. First, try to find the handler by its full name.
        let handler = if self.handlers.contains_key(target) {
            self.handlers.get(target)
        } else if let Some(tagged_names) = self.handler_tags.get(target) {
            if tagged_names.len() > 1 {
                return Err(RuntimeError::AmbiguousHandlerName(
                    target.to_string(),
                    tagged_names.clone(),
                ));
            }
            tagged_names.get(0).and_then(|name| self.handlers.get(name))
        } else {
            None
        };

        let handler = handler.ok_or_else(|| RuntimeError::HandlerNotFound(target.to_string()))?;

        let mut ctx = ExecutionContext {
            client: Some(self.client.clone().unwrap()),
            namespace: self.namespace.clone().unwrap(),
            project: self.project.clone().unwrap(),
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
                Err(e)
            }
        }
    }
}

pub fn create_stub_builder() -> ExecutorBuilder<AutodiffBackendStub> {
    ExecutorBuilder::new()
}

// --- Example Handlers ---

#[cfg(test)]
mod test {
    use super::*;
    use burn::backend::{Autodiff, NdArray};
    use burn::prelude::Backend;
    use burn::tensor::backend::AutodiffBackend;
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
        use crate::executor::{Config, ExecutionContext, ExecutorBuilder, StaticPlugin};
        use burn::prelude::Backend;
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
                &self,
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
        impl<B: Backend> StaticPlugin<B> for DerivedExperimentConfig {
            fn build(builder: &mut ExecutorBuilder<B>) {
                fn wrapped_test_associated_system<B: Backend>(
                    Config(config): Config<DerivedExperimentConfig>,
                    ctx: &ExecutionContext<B>,
                ) -> anyhow::Result<()> {
                    DerivedExperimentConfig::test_associated_system(&config, ctx)
                }
                builder.add_handler(("test_associated_system", wrapped_test_associated_system));
            }
        }
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
        _context: &ExecutionContext<B>,
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
    fn train_model<B: Backend>(config: Config<SomeExperimentConfig>, _model: Model<TestModel<B>>) {
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
        // Create an initial context for a specific experiment run
        let mut builder = Executor::<Back>::builder();

        build_executor(&mut builder);

        let client = BurnCentral::builder(BurnCentralCredentials::new(
            "8543d2e1-1b48-4205-9d5e-3cd282126ec1",
        ))
        .with_endpoint("http://localhost:9001")
        .build()
        .expect("Failed to build BurnCentral client");

        let executor = builder.build(client, "test_namespace", "test_project");

        let override_json = serde_json::to_string(&SomeExperimentConfig {
            param1: 42.0,
            param2: "example".to_string(),
        })
        .expect("Failed to serialize config");

        executor
            .run("log_model2", vec![Default::default()], Some(override_json))
            .expect("Execution failed");
    }

    #[test]
    fn test_stub_executor() {
        // Create a stub executor builder
        let mut builder = create_stub_builder();

        // Add handlers to the stub executor
        build_executor(&mut builder);

        // Build the stub executor
        let executor = builder.build_stub();

        for target in executor.targets() {
            println!("Registered target: {}", target);
        }
    }

    pub struct CustomSystemStruct;

    impl<B: AutodiffBackend> System<B> for CustomSystemStruct {
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
        exec.add_handler(sys!(train_model))
            .add_handler(("log_model2", log_model2))
            .add_handler(("Some_name", log_completion))
            .add_handler(CustomSystemStruct)
            .init_resource::<TestModel<Back>>();
    }

    // This would be the function that the user implements to build the executor in their application
    fn build_executor2<B: AutodiffBackend>(exec: &mut ExecutorBuilder<B>) {
        register_handlers!(exec,
            train_model => "train_model",
            log_model2 => "log_model2",
            log_completion => "log_completion",
            CustomSystemStruct => "custom_system_struct",
            |config: Config<SomeExperimentConfig>, _model: Model<TestModel<B>>, ctx: &ExecutionContext<B>| {
                log_model2(ctx.experiment().unwrap(), config, ctx)
            } => "as"
        );
    }
}
