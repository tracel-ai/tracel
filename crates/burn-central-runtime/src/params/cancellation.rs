use burn_central_experiment::CancelToken;

use crate::{executor::ExecutionContext, params::RoutineParam};

impl RoutineParam<ExecutionContext> for CancelToken {
    type Item<'new> = CancelToken;

    fn try_retrieve(ctx: &ExecutionContext) -> anyhow::Result<Self::Item<'_>> {
        Ok(ctx.cancel_token().clone())
    }
}
