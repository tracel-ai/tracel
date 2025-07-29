use crate::type_name::fn_type_name;
use anyhow::Result;
use burn::prelude::{Backend, Module};

use crate::backend::AutodiffBackendStub;
use burn::tensor::backend::AutodiffBackend;
use burn_central_client::experiment::{
    ExperimentConfig, ExperimentRun, ExperimentTrackerError, deserialize_and_merge_with_default,
};
use burn_central_client::record::ArtifactKind;
use burn_central_client::{BurnCentral, BurnCentralError};
use derive_more::{Deref, From};
use std::collections::HashMap;
use std::marker::PhantomData;
use variadics_please::all_tuples;

/// This trait defines how parameters for a routine are retrieved from the execution context.
pub trait RoutineParam<B: Backend>: Sized {
    type Item<'new>;

    /// This method retrieves the parameter from the context.
    fn retrieve(ctx: &ExecutionContext<B>) -> Self::Item<'_> {
        Self::try_retrieve(ctx).unwrap()
    }

    /// This method attempts to retrieve the parameter from the context, returning an error if it fails.
    fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>>;
}

impl<B: Backend> RoutineParam<B> for &ExecutionContext<B> {
    type Item<'new> = &'new ExecutionContext<B>;

    fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
        Ok(ctx)
    }
}

#[derive(From, Deref)]
pub struct Cfg<T>(pub T);

impl<B: Backend, C: ExperimentConfig> RoutineParam<B> for Cfg<C> {
    type Item<'new> = Cfg<C>;

    fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
        let cfg = ctx.get_merged_config();
        Ok(Cfg(cfg))
    }
}

impl<B: Backend, M: Module<B> + Default> RoutineParam<B> for Model<M> {
    type Item<'new> = Model<M>;

    fn try_retrieve(_ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
        // Assuming we have a way to get the model from the context
        // For simplicity, let's just return a default model here
        let model = M::default();
        Ok(Model(model))
    }
}

#[derive(Clone, Debug, Deref, From)]
pub struct MultiDevice<B: Backend>(pub Vec<B::Device>);

impl<B: Backend> RoutineParam<B> for MultiDevice<B> {
    type Item<'new> = MultiDevice<B>;

    fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
        Ok(MultiDevice(ctx.devices.clone()))
    }
}

impl<B: Backend> RoutineParam<B> for &ExperimentRun {
    type Item<'new> = &'new ExperimentRun;

    fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
        ctx.experiment()
            .ok_or_else(|| anyhow::anyhow!("Experiment run not found"))
    }
}

impl<B: Backend, P: RoutineParam<B>> RoutineParam<B> for Option<P> {
    type Item<'new> = Option<P::Item<'new>>;

    fn try_retrieve(ctx: &ExecutionContext<B>) -> Result<Self::Item<'_>> {
        match P::try_retrieve(ctx) {
            Ok(item) => Ok(Some(item)),
            Err(_) => Ok(None),
        }
    }
}

// for all tuples
macro_rules! impl_routine_param_tuple {
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
        impl<B: Backend, $($P: RoutineParam<B>),*> RoutineParam<B> for ($($P,)*) {
            type Item<'new> = ($($P::Item<'new>,)*);

            fn try_retrieve<'r>(ctx: &'r ExecutionContext<B>) -> Result<Self::Item<'r>> {
                Ok((
                    $(<$P as RoutineParam<B>>::try_retrieve(ctx)?,)*
                ))
            }
        }
    };
}

all_tuples!(impl_routine_param_tuple, 0, 16, P);

/// This trait defines how a specific return type (Output) from a handler
/// is processed and potentially stored back into the ExecutionContext.
pub trait RoutineOutput<B: Backend>: Sized + Send + 'static {
    /// This method takes the owned output and the mutable ExecutionContext,
    /// allowing the output to modify the context.
    fn apply_output(self, ctx: &mut ExecutionContext<B>) -> Result<Self>;
}
/// This trait is a marker for outputs that are specifically related to training routines.
pub trait TrainOutput<B: Backend>: RoutineOutput<B> {}

impl<B: Backend> RoutineOutput<B> for () {
    fn apply_output(self, _ctx: &mut ExecutionContext<B>) -> Result<Self> {
        Ok(())
    }
}

impl<T, E, B: Backend> TrainOutput<B> for core::result::Result<T, E>
where
    T: TrainOutput<B>,
    E: std::fmt::Display + Send + Sync + 'static,
{
}

impl<T, E, B: Backend> RoutineOutput<B> for core::result::Result<T, E>
where
    T: RoutineOutput<B>,
    E: std::fmt::Display + Send + Sync + 'static,
{
    fn apply_output(self, ctx: &mut ExecutionContext<B>) -> Result<Self> {
        match self {
            Ok(output) => Ok(Ok(output.apply_output(ctx)?)),
            Err(e) => {
                // Log the error or handle it as needed
                Err(anyhow::anyhow!(e.to_string()))
            }
        }
    }
}

#[derive(Clone, From, Deref)]
pub struct Model<M>(M);
impl<B: Backend, M: Module<B> + 'static> TrainOutput<B> for Model<M> {}
impl<B: Backend, M: Module<B> + 'static> RoutineOutput<B> for Model<M> {
    fn apply_output(self, ctx: &mut ExecutionContext<B>) -> Result<Self> {
        if let Some(experiment) = ctx.experiment.as_ref() {
            experiment.try_log_artifact(
                "model",
                ArtifactKind::Model,
                self.0.clone().into_record(),
            )?;
        }
        Ok(self)
    }
}

#[diagnostic::on_unimplemented(message = "`{Self}` is not a routine", label = "invalid routine")]
pub trait Routine<B: Backend>: Send + Sync + 'static {
    type Out;

    fn name(&self) -> &str;
    fn run(&self, ctx: &mut ExecutionContext<B>) -> Result<Self::Out, RuntimeError>;
}

pub type BoxedRoutine<B, Out> = Box<dyn Routine<B, Out = Out>>;
pub type ExecutorRoutine<B> = BoxedRoutine<B, ()>;

pub type RoutineParamItem<'ctx, B, P> = <P as RoutineParam<B>>::Item<'ctx>;

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a valid routine",
    label = "invalid routine"
)]
pub trait RoutineParamFunction<B: Backend, Marker>: Send + Sync + 'static {
    type Out;
    type Param: RoutineParam<B>;

    fn run(&self, param_value: RoutineParamItem<B, Self::Param>)
    -> Result<Self::Out, RuntimeError>;
}

macro_rules! impl_routine_function {
    ($($param: ident),*) => {
        #[expect(
            clippy::allow_attributes,
            reason = "This is within a macro, and as such, the below lints may not always apply."
        )]
        #[allow(
            non_snake_case,
            reason = "Certain variable names are provided by the caller, not by us."
        )]
        impl<B: Backend, Out, Func, $($param: RoutineParam<B>),*> RoutineParamFunction<B, fn($($param,)*) -> Out> for Func
        where
            Func: Send + Sync + 'static,
            for <'a> &'a Func:
                Fn($($param),*) -> Out +
                Fn($(RoutineParamItem<B, $param>),*) -> Out,
            Out: 'static,
        {
            type Out = Out;
            type Param = ($($param,)*);
            #[inline]
            fn run(&self, param_value: RoutineParamItem<B, ($($param,)*)>) -> Result<Self::Out, RuntimeError> {
                #[expect(
                    clippy::allow_attributes,
                    reason = "This is within a macro, and as such, the below lints may not always apply."
                )]
                #[allow(clippy::too_many_arguments)]
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

all_tuples!(impl_routine_function, 0, 16, F);

#[doc(hidden)]
pub struct IsFunctionRoutine;

pub struct FunctionRoutine<Marker, F> {
    func: F,
    name: String,
    _marker: PhantomData<fn() -> Marker>,
}

impl<Marker, F> FunctionRoutine<Marker, F> {
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

impl<Marker, F: Clone> Clone for FunctionRoutine<Marker, F> {
    fn clone(&self) -> Self {
        FunctionRoutine {
            func: self.func.clone(),
            name: self.name.clone(),
            _marker: PhantomData,
        }
    }
}

impl<B, Marker, F> IntoRoutine<B, F::Out, (IsFunctionRoutine, B, Marker)> for F
where
    B: Backend,
    Marker: 'static,
    F: RoutineParamFunction<B, Marker>,
{
    type Routine = FunctionRoutine<Marker, F>;

    fn into_routine(func: Self) -> Self::Routine {
        FunctionRoutine {
            func,
            name: fn_type_name::<F>(),
            _marker: PhantomData,
        }
    }
}

impl<B, Marker, F> Routine<B> for FunctionRoutine<Marker, F>
where
    B: Backend,
    Marker: 'static,
    F: RoutineParamFunction<B, Marker>,
{
    type Out = F::Out;

    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn run(&self, ctx: &mut ExecutionContext<B>) -> Result<Self::Out, RuntimeError> {
        let params = F::Param::try_retrieve(ctx).map_err(|e| {
            RuntimeError::HandlerFailed(anyhow::anyhow!("Failed to retrieve parameters: {}", e))
        })?;
        let output = self.func.run(params)?;
        Ok(output)
    }
}

impl<B: Backend, T: Routine<B>> IntoRoutine<B, T::Out, ()> for T {
    type Routine = T;
    fn into_routine(this: Self) -> Self::Routine {
        this
    }
}

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a valid routine with output `{Output}`",
    label = "invalid routine"
)]
pub trait IntoRoutine<B: Backend, Output, Marker>: Sized {
    type Routine: Routine<B, Out = Output>;

    #[allow(clippy::wrong_self_convention)]
    fn into_routine(this: Self) -> Self::Routine;

    /// Assigns a custom name to a routine, overriding the default.
    ///
    /// The default name for a function routine is derived from its type, which is unique.
    /// This modifier allows you to register the same routine function multiple times
    /// under different names, which can be useful for creating distinct stages in a
    /// workflow that use the same logic.
    fn with_name(self, name: impl Into<String>) -> IntoNamedRoutine<Self> {
        IntoNamedRoutine {
            routine: self,
            name: name.into(),
        }
    }
}

/// A wrapper for an `IntoRoutine`-implementing type that holds a custom name.
/// This is constructed by the `.with_name()` method from the `IntoRoutine` trait.
#[derive(Clone)]
pub struct IntoNamedRoutine<S> {
    routine: S,
    name: String,
}

/// A `Routine` that wraps another `Routine` to override its name.
pub struct NamedRoutine<S> {
    inner: S,
    name: String,
}

impl<S, B> Routine<B> for NamedRoutine<S>
where
    S: Routine<B>,
    B: Backend,
{
    type Out = S::Out;

    fn name(&self) -> &str {
        &self.name
    }

    fn run(&self, ctx: &mut ExecutionContext<B>) -> Result<Self::Out, RuntimeError> {
        self.inner.run(ctx)
    }
}

#[doc(hidden)]
pub struct IsNamedRoutine;
// Implements `IntoRoutine` for the `Named` wrapper. This allows a named routines to be
// passed to methods like `add_handler`.
impl<B, O, M, S> IntoRoutine<B, O, (IsNamedRoutine, B, O, M)> for IntoNamedRoutine<S>
where
    B: Backend,
    S: IntoRoutine<B, O, M>,
{
    type Routine = NamedRoutine<S::Routine>;

    fn into_routine(this: Self) -> Self::Routine {
        NamedRoutine {
            inner: IntoRoutine::into_routine(this.routine),
            name: this.name,
        }
    }
}

impl<B, O, M, S, N> IntoRoutine<B, O, (IsNamedRoutine, B, O, M, N)> for (N, S)
where
    B: Backend,
    S: IntoRoutine<B, O, M>,
    N: Into<String>,
{
    type Routine = NamedRoutine<S::Routine>;

    fn into_routine(this: Self) -> Self::Routine {
        let (name, routines) = this;
        NamedRoutine {
            inner: IntoRoutine::into_routine(routines),
            name: name.into(),
        }
    }
}

struct ExecutorRoutineWrapper<S, B>(S, PhantomData<fn() -> B>);
impl<S, B, Output> ExecutorRoutineWrapper<S, B>
where
    S: Routine<B, Out = Output>,
    B: Backend,
    Output: RoutineOutput<B>,
{
    pub fn new(routine: S) -> Self {
        ExecutorRoutineWrapper(routine, PhantomData)
    }
}

impl<B, S, Output> Routine<B> for ExecutorRoutineWrapper<S, B>
where
    B: Backend,
    S: Routine<B, Out = Output>,
    Output: RoutineOutput<B>,
{
    type Out = ();

    fn name(&self) -> &str {
        self.0.name()
    }

    fn run(&self, ctx: &mut ExecutionContext<B>) -> Result<Self::Out, RuntimeError> {
        match self.0.run(ctx) {
            Ok(output) => {
                output.apply_output(ctx).map_err(|e| {
                    log::error!("Failed to apply output: {e}");
                    RuntimeError::HandlerFailed(anyhow::anyhow!("Failed to apply output: {}", e))
                })?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

// --- Custom Error Type ---
#[derive(thiserror::Error, Debug)]
pub enum RuntimeError {
    #[error("Handler '{0}' not found")]
    HandlerNotFound(String),
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
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, strum::Display, strum::EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum ActionKind {
    Train,
    // Infer,
    // Eval,
    // Test,
    // #[strum(serialize = "custom({0})")]
    // Custom(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TargetId {
    kind: ActionKind,
    name: String,
}

impl std::fmt::Display for TargetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.kind, self.name)
    }
}

pub struct ExecutorBuilder<B: AutodiffBackend> {
    executor: Executor<B>,
}

impl<B: AutodiffBackend> ExecutorBuilder<B> {
    fn new() -> Self {
        Self {
            executor: Executor {
                client: None,
                namespace: None,
                project: None,
                handlers: HashMap::new(),
            },
        }
    }

    fn register<M, O: RoutineOutput<B>>(
        &mut self,
        kind: ActionKind,
        name: impl Into<String>,
        handler: impl IntoRoutine<B, O, M>,
    ) -> &mut Self {
        let wrapper = ExecutorRoutineWrapper::new(IntoRoutine::into_routine(handler));
        let routine = Box::new(wrapper);
        let routine_name = routine.name();

        let target_id = TargetId {
            kind,
            name: name.into(),
        };

        log::debug!("Registering handler '{routine_name}' for target: {target_id}");

        self.executor.handlers.insert(target_id, routine);
        self
    }

    pub fn train<M, O: TrainOutput<B>>(
        &mut self,
        name: impl Into<String>,
        handler: impl IntoRoutine<B, O, M>,
    ) -> &mut Self {
        self.register(ActionKind::Train, name, handler);
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

    pub fn build_stub(self) -> Executor<B> {
        self.executor
    }
}

pub struct Executor<B: Backend> {
    client: Option<BurnCentral>,
    namespace: Option<String>,
    project: Option<String>,
    handlers: HashMap<TargetId, ExecutorRoutine<B>>,
}

impl<B: AutodiffBackend> Executor<B> {
    pub fn builder() -> ExecutorBuilder<B> {
        ExecutorBuilder::new()
    }

    pub fn targets(&self) -> Vec<TargetId> {
        self.handlers.keys().cloned().collect()
    }

    pub fn run(
        &self,
        kind: ActionKind,
        name: impl AsRef<str>,
        devices: impl IntoIterator<Item = B::Device>,
        config_override: Option<String>,
    ) -> Result<(), RuntimeError> {
        let target = name.as_ref();

        let target_id = TargetId {
            kind,
            name: target.to_string(),
        };

        let handler = self.handlers.get(&target_id).ok_or_else(|| {
            log::error!("Handler not found for target: {target}");
            RuntimeError::HandlerNotFound(target.to_string())
        })?;

        log::debug!("Starting Execution for Target: {target}");

        let mut ctx = ExecutionContext {
            client: Some(self.client.clone().unwrap()),
            namespace: self.namespace.clone().unwrap(),
            project: self.project.clone().unwrap(),
            config_override,
            devices: devices.into_iter().collect(),
            experiment: None,
        };

        let config = ctx.config_override.as_deref().unwrap_or("{}");

        if let Some(client) = &mut ctx.client {
            let code_version = option_env!("BURN_CENTRAL_CODE_VERSION")
                .unwrap_or("unknown")
                .to_string();
            log::debug!("Using Burn Central client with code version: {code_version}");

            log::info!(
                "Starting experiment for target: {} in namespace: {}, project: {}",
                target,
                ctx.namespace,
                ctx.project
            );
            let experiment = client.start_experiment(&ctx.namespace, &ctx.project, &config)?;
            ctx.experiment = Some(experiment);
        }

        let result = handler.run(&mut ctx);

        match result {
            Ok(_) => {
                if let Some(experiment) = ctx.experiment {
                    experiment.finish()?;
                    log::info!("Experiment run completed successfully.");
                }
                log::debug!("Handler {target} executed successfully.");

                Ok(())
            }
            Err(e) => {
                log::error!("Error executing handler '{target}': {e}");
                if let Some(experiment) = ctx.experiment {
                    experiment.fail(e.to_string())?;
                    log::error!("Experiment run failed: {e}");
                }
                Err(e)
            }
        }
    }
}

pub fn create_stub_builder() -> ExecutorBuilder<AutodiffBackendStub> {
    ExecutorBuilder::new()
}
