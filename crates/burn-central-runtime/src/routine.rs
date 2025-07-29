use crate::ExecutionContext;
use crate::error::RuntimeError;
use crate::param::RoutineParam;
use crate::type_name::fn_type_name;
use burn::prelude::Backend;
use std::marker::PhantomData;
use variadics_please::all_tuples;
use crate::output::RoutineOutput;

#[diagnostic::on_unimplemented(message = "`{Self}` is not a routine", label = "invalid routine")]
pub trait Routine<B: Backend>: Send + Sync + 'static {
    type Out;

    fn name(&self) -> &str;
    fn run(&self, ctx: &mut ExecutionContext<B>) -> anyhow::Result<Self::Out, RuntimeError>;
}

pub type RoutineParamItem<'ctx, B, P> = <P as RoutineParam<B>>::Item<'ctx>;

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a valid routine",
    label = "invalid routine"
)]
pub trait RoutineParamFunction<B: Backend, Marker>: Send + Sync + 'static {
    type Out;
    type Param: RoutineParam<B>;

    fn run(
        &self,
        param_value: RoutineParamItem<B, Self::Param>,
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

impl<B, Marker, F> IntoRoutine<B, F::Out, (IsFunctionRoutine, Marker)> for F
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

    fn run(&self, ctx: &mut ExecutionContext<B>) -> anyhow::Result<Self::Out, RuntimeError> {
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
    fn with_name(self, name: impl Into<String>) -> IntoNamedRoutine<B, Self> {
        IntoNamedRoutine {
            routine: self,
            name: name.into(),
            marker: Default::default(),
        }
    }
}

/// A wrapper for an `IntoRoutine`-implementing type that holds a custom name.
/// This is constructed by the `.with_name()` method from the `IntoRoutine` trait.
#[derive(Clone)]
pub struct IntoNamedRoutine<B, S> {
    routine: S,
    name: String,
    marker: PhantomData<fn() -> B>,
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

    fn run(&self, ctx: &mut ExecutionContext<B>) -> anyhow::Result<Self::Out, RuntimeError> {
        self.inner.run(ctx)
    }
}

#[doc(hidden)]
pub struct IsNamedRoutine;
impl<B, O, M, S> IntoRoutine<B, O, (IsNamedRoutine, O, M)> for IntoNamedRoutine<B, S>
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

impl<B, O, M, S, N> IntoRoutine<B, O, (IsNamedRoutine, O, N, M)> for (N, S)
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

/// A wrapper for a routine that is used by the executor to run routines.
pub struct ExecutorRoutineWrapper<S, B>(S, PhantomData<fn() -> B>);
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

    fn run(&self, ctx: &mut ExecutionContext<B>) -> anyhow::Result<Self::Out, RuntimeError> {
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

pub type BoxedRoutine<B, Out> = Box<dyn Routine<B, Out = Out>>;
