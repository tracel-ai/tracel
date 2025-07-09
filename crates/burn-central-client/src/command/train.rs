use crate::experiment::ExperimentRun;
use burn::{config::Config, module::Module, tensor::backend::Backend};

#[derive(Debug, Clone)]
pub struct MultiDevice<B: Backend>(pub Vec<B::Device>);

#[derive(Clone)]
pub struct TrainCommandContext<'a, B: Backend> {
    experiment: &'a ExperimentRun,
    devices: Vec<B::Device>,
    config: String,
}

impl<'a, B: Backend> TrainCommandContext<'a, B> {
    pub fn new(experiment: &'a ExperimentRun, devices: Vec<B::Device>, config: String) -> Self {
        Self {
            experiment,
            devices,
            config,
        }
    }
}

trait FromTrainCommandContext<'a, B: Backend> {
    fn from_context(context: &'a TrainCommandContext<'a, B>) -> Self;
}

impl<'a, B: Backend> FromTrainCommandContext<'a, B> for &'a ExperimentRun {
    fn from_context(context: &'a TrainCommandContext<'a, B>) -> Self {
        context.experiment
    }
}

impl<'a, B: Backend> FromTrainCommandContext<'a, B> for MultiDevice<B> {
    fn from_context(context: &'a TrainCommandContext<'a, B>) -> Self {
        MultiDevice(context.devices.clone())
    }
}

impl<B: Backend> std::ops::Deref for MultiDevice<B> {
    type Target = Vec<B::Device>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a, B: Backend, T: Config> FromTrainCommandContext<'a, B> for T {
    fn from_context(context: &'a TrainCommandContext<'a, B>) -> Self {
        T::load_binary(context.config.as_bytes()).expect("Config should be loaded")
    }
}

pub trait TrainCommandHandler<'a, B: Backend, T, M: Module<B>, E: Into<Box<dyn std::error::Error>>>
{
    fn call(self, context: &'a TrainCommandContext<'a, B>) -> Result<M, E>;
}

macro_rules! impl_train_command_handler {
    ($($T:ident),*) => {
        impl<'a, F, M, B, E, $($T),*> TrainCommandHandler<'a, B, ($($T,)*), M, E> for F
        where
            F: Fn($($T),*) -> Result<M, E>,
            M: Module<B>,
            B: Backend,
            E: Into<Box<dyn std::error::Error>>,
            $($T: FromTrainCommandContext<'a, B>),*
        {
            fn call(self, _context: &'a TrainCommandContext<'a, B>) -> Result<M, E> {
                (self)($($T::from_context(_context)),*)
            }
        }
    };
}

impl_train_command_handler!();
impl_train_command_handler!(T1);
impl_train_command_handler!(T1, T2);
impl_train_command_handler!(T1, T2, T3);
