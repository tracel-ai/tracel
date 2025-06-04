use burn::{config::Config, module::Module, tensor::backend::Backend};

use crate::client::BurnCentralClient;

#[derive(Debug, Clone)]
pub struct MultiDevice<B: Backend>(pub Vec<B::Device>);

#[derive(Debug, Clone)]
pub struct TrainCommandContext<B: Backend> {
    client: BurnCentralClient,
    devices: Vec<B::Device>,
    config: String,
}

impl<B: Backend> TrainCommandContext<B> {
    pub fn new(client: BurnCentralClient, devices: Vec<B::Device>, config: String) -> Self {
        Self {
            client,
            devices,
            config,
        }
    }

    pub fn client(&mut self) -> &mut BurnCentralClient {
        &mut self.client
    }

    pub fn devices(&mut self) -> &mut Vec<B::Device> {
        &mut self.devices
    }

    pub fn config(&self) -> &str {
        &self.config
    }
}

trait FromTrainCommandContext<B: Backend> {
    fn from_context(context: &TrainCommandContext<B>) -> Self;
}

impl<B: Backend> FromTrainCommandContext<B> for BurnCentralClient {
    fn from_context(context: &TrainCommandContext<B>) -> Self {
        context.client.clone()
    }
}

impl<B: Backend> FromTrainCommandContext<B> for MultiDevice<B> {
    fn from_context(context: &TrainCommandContext<B>) -> Self {
        MultiDevice(context.devices.clone())
    }
}

impl<B: Backend> IntoIterator for MultiDevice<B> {
    type Item = B::Device;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<B: Backend, T: Config> FromTrainCommandContext<B> for T {
    fn from_context(context: &TrainCommandContext<B>) -> Self {
        T::load_binary(context.config.as_bytes()).expect("Config should be loaded")
    }
}

pub trait TrainCommandHandler<B: Backend, T, M: Module<B>, E: Into<Box<dyn std::error::Error>>> {
    fn call(self, context: TrainCommandContext<B>) -> Result<M, E>;
}

impl<F, M, B, E: Into<Box<dyn std::error::Error>>> TrainCommandHandler<B, (), M, E> for F
where
    F: Fn() -> Result<M, E>,
    M: Module<B>,
    B: Backend,
{
    fn call(self, _context: TrainCommandContext<B>) -> Result<M, E> {
        (self)()
    }
}

impl<F, T, M, B, E: Into<Box<dyn std::error::Error>>> TrainCommandHandler<B, (T,), M, E> for F
where
    F: Fn(T) -> Result<M, E>,
    T: FromTrainCommandContext<B>,
    M: Module<B>,
    B: Backend,
{
    fn call(self, context: TrainCommandContext<B>) -> Result<M, E> {
        (self)(T::from_context(&context))
    }
}

impl<F, T1, T2, M, B, E: Into<Box<dyn std::error::Error>>> TrainCommandHandler<B, (T1, T2), M, E>
    for F
where
    F: Fn(T1, T2) -> Result<M, E>,
    T1: FromTrainCommandContext<B>,
    T2: FromTrainCommandContext<B>,
    M: Module<B>,
    B: Backend,
{
    fn call(self, context: TrainCommandContext<B>) -> Result<M, E> {
        (self)(T1::from_context(&context), T2::from_context(&context))
    }
}

impl<F, T1, T2, T3, M, B, E: Into<Box<dyn std::error::Error>>>
    TrainCommandHandler<B, (T1, T2, T3), M, E> for F
where
    F: Fn(T1, T2, T3) -> Result<M, E>,
    T1: FromTrainCommandContext<B>,
    T2: FromTrainCommandContext<B>,
    T3: FromTrainCommandContext<B>,
    M: Module<B>,
    B: Backend,
{
    fn call(self, context: TrainCommandContext<B>) -> Result<M, E> {
        (self)(
            T1::from_context(&context),
            T2::from_context(&context),
            T3::from_context(&context),
        )
    }
}
