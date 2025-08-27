use crate::error::RuntimeError;
use crate::output::RoutineOutput;
use crate::param::RoutineParam;
use crate::type_name::fn_type_name;
use std::marker::PhantomData;
use variadics_please::all_tuples;

#[diagnostic::on_unimplemented(message = "`{Self}` is not a routine", label = "invalid routine")]
pub trait Routine<Ctx>: Send + Sync + 'static {
    type Out;

    fn name(&self) -> &str;
    fn run(&self, ctx: &mut Ctx) -> anyhow::Result<Self::Out, RuntimeError>;
}

pub type RoutineParamItem<'ctx, Ctx, P> = <P as RoutineParam<Ctx>>::Item<'ctx>;

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a valid routine",
    label = "invalid routine"
)]
pub trait RoutineParamFunction<Ctx, Marker>: Send + Sync + 'static {
    type Out;
    type Param: RoutineParam<Ctx>;

    fn run(
        &self,
        param_value: RoutineParamItem<Ctx, Self::Param>,
    ) -> anyhow::Result<Self::Out, RuntimeError>;
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
        impl<Ctx, Out, Func, $($param: RoutineParam<Ctx>),*> RoutineParamFunction<Ctx, fn($($param,)*) -> Out> for Func
        where
            Func: Send + Sync + 'static,
            for <'a> &'a Func:
                Fn($($param),*) -> Out +
                Fn($(RoutineParamItem<Ctx, $param>),*) -> Out,
            Out: 'static,
            Ctx: 'static,
        {
            type Out = Out;
            type Param = ($($param,)*);
            #[inline]
            fn run(&self, param_value: RoutineParamItem<Ctx, ($($param,)*)>) -> Result<Self::Out, RuntimeError> {
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

impl<Ctx, Marker, F> IntoRoutine<Ctx, F::Out, (IsFunctionRoutine, Marker)> for F
where
    Marker: 'static,
    F: RoutineParamFunction<Ctx, Marker>,
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

impl<Ctx, Marker, F> Routine<Ctx> for FunctionRoutine<Marker, F>
where
    Marker: 'static,
    F: RoutineParamFunction<Ctx, Marker>,
{
    type Out = F::Out;

    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn run(&self, ctx: &mut Ctx) -> anyhow::Result<Self::Out, RuntimeError> {
        let params = <F::Param as RoutineParam<Ctx>>::try_retrieve(ctx).map_err(|e| {
            RuntimeError::HandlerFailed(anyhow::anyhow!("Failed to retrieve parameters: {}", e))
        })?;
        let output = self.func.run(params)?;
        Ok(output)
    }
}

impl<Ctx, T: Routine<Ctx>> IntoRoutine<Ctx, T::Out, ()> for T {
    type Routine = T;
    fn into_routine(this: Self) -> Self::Routine {
        this
    }
}

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a valid routine with output `{Output}`",
    label = "invalid routine"
)]
pub trait IntoRoutine<Ctx, Output, Marker>: Sized {
    type Routine: Routine<Ctx, Out = Output>;

    #[allow(clippy::wrong_self_convention)]
    fn into_routine(this: Self) -> Self::Routine;

    fn with_name(self, name: impl Into<String>) -> IntoNamedRoutine<Ctx, Self> {
        IntoNamedRoutine {
            routine: self,
            name: name.into(),
            marker: Default::default(),
        }
    }
}

#[derive(Clone)]
pub struct IntoNamedRoutine<Ctx, S> {
    routine: S,
    name: String,
    marker: PhantomData<fn(Ctx)>,
}

pub struct NamedRoutine<S> {
    inner: S,
    name: String,
}

impl<Ctx, S> Routine<Ctx> for NamedRoutine<S>
where
    S: Routine<Ctx>,
{
    type Out = S::Out;

    fn name(&self) -> &str {
        &self.name
    }

    fn run(&self, ctx: &mut Ctx) -> anyhow::Result<Self::Out, RuntimeError> {
        self.inner.run(ctx)
    }
}

#[doc(hidden)]
pub struct IsNamedRoutine;
impl<Ctx, O, M, S> IntoRoutine<Ctx, O, (IsNamedRoutine, O, M)> for IntoNamedRoutine<Ctx, S>
where
    S: IntoRoutine<Ctx, O, M>,
{
    type Routine = NamedRoutine<S::Routine>;

    fn into_routine(this: Self) -> Self::Routine {
        NamedRoutine {
            inner: IntoRoutine::into_routine(this.routine),
            name: this.name,
        }
    }
}

impl<Ctx, O, M, S, N> IntoRoutine<Ctx, O, (IsNamedRoutine, O, N, M)> for (N, S)
where
    S: IntoRoutine<Ctx, O, M>,
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

pub struct ExecutorRoutineWrapper<S, Ctx>(S, PhantomData<Ctx>);

impl<S, Ctx, Output> ExecutorRoutineWrapper<S, Ctx>
where
    S: Routine<Ctx, Out = Output>,
    // This assumes `RoutineOutput` is also made generic over `Ctx`.
    Output: RoutineOutput<Ctx>,
{
    pub fn new(routine: S) -> Self {
        ExecutorRoutineWrapper(routine, PhantomData)
    }
}

impl<Ctx, S, Output> Routine<Ctx> for ExecutorRoutineWrapper<S, Ctx>
where
    S: Routine<Ctx, Out = Output>,
    // This assumes `RoutineOutput` is also made generic over `Ctx`.
    Output: RoutineOutput<Ctx>,
    Ctx: std::marker::Send + std::marker::Sync + 'static,
{
    type Out = ();

    fn name(&self) -> &str {
        self.0.name()
    }

    fn run(&self, ctx: &mut Ctx) -> anyhow::Result<Self::Out, RuntimeError> {
        match self.0.run(ctx) {
            Ok(output) => {
                output.apply_output(ctx).map_err(|e| {
                    // Assuming a logger is available, e.g., from the `log` crate.
                    // log::error!("Failed to apply output: {e}");
                    RuntimeError::HandlerFailed(anyhow::anyhow!("Failed to apply output: {}", e))
                })?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

/// The boxed routine type alias is updated to include `Ctx`.
pub type BoxedRoutine<Ctx, Out> = Box<dyn Routine<Ctx, Out = Out>>;
