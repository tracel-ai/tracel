use crate::executor::ExecutionContext;
use crate::params::default::Model;
use tracel_artifact::bundle::BundleEncode;
use tracel_experiment::ArtifactKind;
use std::fmt::Display;

/// This trait defines how a specific return type (Output) from a handler apply its effects to the execution context.
pub trait RoutineOutput<Ctx>: Sized + Send + 'static {
    /// This method takes the owned output and the mutable ExecutionContext,
    /// allowing the output to modify the context.
    fn apply_output(self, ctx: &mut Ctx) -> anyhow::Result<()>;
}

pub trait ExperimentOutput: RoutineOutput<ExecutionContext> {}

/// This trait is a marker for outputs that are specifically related to training routines.
pub trait TrainOutput: ExperimentOutput {}

/// This implementation is for the case where the output is simply `()`, meaning no output to apply.
impl RoutineOutput<ExecutionContext> for () {
    fn apply_output(self, _ctx: &mut ExecutionContext) -> anyhow::Result<()> {
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

impl<M: BundleEncode + Send + 'static> RoutineOutput<ExecutionContext> for Model<M> {
    fn apply_output(self, ctx: &mut ExecutionContext) -> anyhow::Result<()> {
        if let Some(experiment) = ctx.experiment() {
            experiment.save_artifact("model", ArtifactKind::Model, self.0, &Default::default())?;
        }
        Ok(())
    }
}

impl<M: BundleEncode + Send + 'static> ExperimentOutput for Model<M> {}

impl ExperimentOutput for () {}

/// --- TrainOutput ---
impl<M: BundleEncode + Send + 'static> TrainOutput for Model<M> {}

impl<T, E> ExperimentOutput for Result<T, E>
where
    E: 'static + Display + Send + Sync,
    T: TrainOutput,
{
}

impl<T, E> TrainOutput for Result<T, E>
where
    T: TrainOutput,
    E: Display + Send + Sync + 'static,
{
}

impl TrainOutput for () {}
