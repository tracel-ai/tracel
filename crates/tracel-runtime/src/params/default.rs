use crate::{executor::ExecutionContext, params::RoutineParam};
use tracel_experiment::ExperimentRun;
use derive_more::{Deref, From};

/// Wrapper around the model returned by a routine.
///
/// This is used to differentiate the model from other return types.
/// Right now the macro force you to return a Model as we expect to be able to log it as a model
/// artifact.
#[derive(Clone, From, Deref)]
pub struct Model<M>(pub M);

impl RoutineParam<ExecutionContext> for &ExecutionContext {
    type Item<'new> = &'new ExecutionContext;

    fn try_retrieve(ctx: &ExecutionContext) -> anyhow::Result<Self::Item<'_>> {
        Ok(ctx)
    }
}

impl RoutineParam<ExecutionContext> for &ExperimentRun {
    type Item<'new> = &'new ExperimentRun;

    fn try_retrieve(ctx: &ExecutionContext) -> anyhow::Result<Self::Item<'_>> {
        ctx.experiment()
            .ok_or_else(|| anyhow::anyhow!("Experiment run not found"))
    }
}

impl<Ctx, P: RoutineParam<Ctx>> RoutineParam<Ctx> for Option<P> {
    type Item<'new>
        = Option<P::Item<'new>>
    where
        Ctx: 'new;

    fn try_retrieve(ctx: &Ctx) -> anyhow::Result<Self::Item<'_>> {
        match P::try_retrieve(ctx) {
            Ok(item) => Ok(Some(item)),
            Err(_) => Ok(None),
        }
    }
}
