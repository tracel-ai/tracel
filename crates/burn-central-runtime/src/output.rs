use crate::ExecutionContext;
use crate::types::Model;
use burn::module::Module;
use burn::prelude::Backend;
use burn_central_client::record::ArtifactKind;
use std::fmt::Display;

/// This trait defines how a specific return type (Output) from a handler apply its effects to the execution context.
pub trait RoutineOutput<B: Backend>: Sized + Send + 'static {
    /// This method takes the owned output and the mutable ExecutionContext,
    /// allowing the output to modify the context.
    fn apply_output(self, ctx: &mut ExecutionContext<B>) -> anyhow::Result<Self>;
}

/// This trait is a marker for outputs that are specifically related to training routines.
pub trait TrainOutput<B: Backend>: RoutineOutput<B> {}

/// This implementation is for the case where the output is simply `()`, meaning no output to apply.
impl<B: Backend> RoutineOutput<B> for () {
    fn apply_output(self, _ctx: &mut ExecutionContext<B>) -> anyhow::Result<Self> {
        Ok(())
    }
}

impl<T, E, B: Backend> RoutineOutput<B> for Result<T, E>
where
    T: RoutineOutput<B>,
    E: Display + Send + Sync + 'static,
{
    fn apply_output(self, ctx: &mut ExecutionContext<B>) -> anyhow::Result<Self> {
        match self {
            Ok(output) => Ok(Ok(output.apply_output(ctx)?)),
            Err(e) => {
                Err(anyhow::anyhow!(e.to_string()))
            }
        }
    }
}

impl<B: Backend, M: Module<B> + 'static> RoutineOutput<B> for Model<M> {
    fn apply_output(self, ctx: &mut ExecutionContext<B>) -> anyhow::Result<Self> {
        if let Some(experiment) = ctx.experiment() {
            experiment.try_log_artifact(
                "model",
                ArtifactKind::Model,
                self.0.clone().into_record(),
            )?;
        }
        Ok(self)
    }
}

// --- TrainOutput ---
impl<B: Backend, M: Module<B> + 'static> TrainOutput<B> for Model<M> {}
impl<T, E, B: Backend> TrainOutput<B> for Result<T, E>
where
    T: TrainOutput<B>,
    E: Display + Send + Sync + 'static,
{
}
