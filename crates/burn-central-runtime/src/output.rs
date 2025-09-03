use crate::inference::InferenceContext;
use crate::types::Model;
use crate::{ExecutionContext, Out};
use burn::module::Module;
use burn::prelude::Backend;
use burn_central_client::record::ArtifactKind;
use std::fmt::Display;

/// This trait defines how a specific return type (Output) from a handler apply its effects to the execution context.
pub trait RoutineOutput<Ctx>: Sized + Send + 'static {
    /// This method takes the owned output and the mutable ExecutionContext,
    /// allowing the output to modify the context.
    fn apply_output(self, ctx: &mut Ctx) -> anyhow::Result<()>;
}

pub trait ExperimentOutput<B: Backend>: RoutineOutput<ExecutionContext<B>> {}

/// This trait is a marker for outputs that are specifically related to training routines.
pub trait TrainOutput<B: Backend>: ExperimentOutput<B> {}

/// This implementation is for the case where the output is simply `()`, meaning no output to apply.
impl<B: Backend> RoutineOutput<ExecutionContext<B>> for () {
    fn apply_output(self, _ctx: &mut ExecutionContext<B>) -> anyhow::Result<()> {
        Ok(())
    }
}

impl<T, E, Ctx> RoutineOutput<Ctx> for Result<T, E>
where
    T: RoutineOutput<Ctx>,
    E: Display + Send + Sync + 'static,
{
    fn apply_output(self, ctx: &mut Ctx) -> anyhow::Result<()> {
        match self {
            Ok(output) => Ok(output.apply_output(ctx)?),
            Err(e) => Err(anyhow::anyhow!(e.to_string())),
        }
    }
}

impl<B: Backend, M: Module<B> + 'static> RoutineOutput<ExecutionContext<B>> for Model<M> {
    fn apply_output(self, ctx: &mut ExecutionContext<B>) -> anyhow::Result<()> {
        if let Some(experiment) = ctx.experiment() {
            experiment.try_log_artifact("model", ArtifactKind::Model, self.0.into_record())?;
        }
        Ok(())
    }
}

impl<B: Backend, M: Module<B> + 'static> ExperimentOutput<B> for Model<M> {}

// --- TrainOutput ---
impl<B: Backend, M: Module<B> + 'static> TrainOutput<B> for Model<M> {}

impl<T, E, B: Backend> ExperimentOutput<B> for Result<T, E>
where
    E: 'static + Display + Send + Sync,
    T: TrainOutput<B>,
{
}

impl<T, E, B: Backend> TrainOutput<B> for Result<T, E>
where
    T: TrainOutput<B>,
    E: Display + Send + Sync + 'static,
{
}

/// This trait is used for outputs that are specifically related to inference routines.
pub trait InferenceOutput<B: Backend, M, S>: RoutineOutput<InferenceContext<B, M, S>> {}

impl<B: Backend, M, S> RoutineOutput<InferenceContext<B, M, S>> for () {
    fn apply_output(self, _ctx: &mut InferenceContext<B, M, S>) -> anyhow::Result<Self> {
        Ok(())
    }
}

impl<B: Backend, M, S> InferenceOutput<B, M, S> for () {}

impl<B: Backend, M, T> RoutineOutput<InferenceContext<B, M, T>> for Out<T>
where
    T: Send + 'static,
{
    fn apply_output(self, ctx: &mut InferenceContext<B, M, T>) -> anyhow::Result<()> {
        use crate::inference::EmitControl;

        match ctx.emitter.emit(self.0) {
            Ok(EmitControl::Continue) | Ok(EmitControl::Stop) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("Failed to emit output: {}", e)),
        }
    }
}

impl<B: Backend, M, T> InferenceOutput<B, M, T> for Out<T> where T: Send + 'static {}

impl<B: Backend, M, O, T, E> InferenceOutput<B, M, O> for Result<T, E>
where
    T: InferenceOutput<B, M, O>,
    E: Display + Send + Sync + 'static,
{
}
