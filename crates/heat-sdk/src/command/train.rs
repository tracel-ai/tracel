use burn::{config::Config, module::Module, tensor::backend::Backend};

use crate::client::HeatClient;

#[derive(Debug, Clone)]
pub struct MultiDevice<B: Backend>(pub Vec<B::Device>);

#[derive(Debug, Clone)]
pub struct TrainCommandContext<B: Backend> {
    client: HeatClient,
    devices: Vec<B::Device>,
    config: String,
}

impl<B: Backend> TrainCommandContext<B> {
    pub fn new(client: HeatClient, devices: Vec<B::Device>, config: String) -> Self {
        Self {
            client,
            devices,
            config,
        }
    }

    pub fn client(&mut self) -> &mut HeatClient {
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

impl<B: Backend> FromTrainCommandContext<B> for HeatClient {
    fn from_context(context: &TrainCommandContext<B>) -> Self {
        println!("Inferred usage of context.client");
        context.client.clone()
    }
}

impl<B: Backend> FromTrainCommandContext<B> for MultiDevice<B> {
    fn from_context(context: &TrainCommandContext<B>) -> Self {
        println!("Inferred usage of context.devices");
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

impl<B: Backend, T: Config> FromTrainCommandContext<B> for T  {
    fn from_context(context: &TrainCommandContext<B>) -> Self {
        println!("Inferred usage of context.config");
        T::load_binary(context.config.as_bytes()).expect("Config should be loaded")
    }
}

pub type TrainResult<M> = Result<M, ()>;

pub trait TrainCommandHandler<B: Backend, T, M: Module<B>> {
    fn call(self, context: TrainCommandContext<B>) -> TrainResult<M>;
}

impl<F, M, B> TrainCommandHandler<B, (), M> for F
where
    F: Fn() -> TrainResult<M>,
    M: Module<B>,
    B: Backend,
{
    fn call(self, _context: TrainCommandContext<B>) -> TrainResult<M> {
        (self)()
    }
}

impl<F, T, M, B> TrainCommandHandler<B, (T,), M> for F
where
    F: Fn(T) -> TrainResult<M>,
    T: FromTrainCommandContext<B>,
    M: Module<B>,
    B: Backend,
{
    fn call(self, context: TrainCommandContext<B>) -> TrainResult<M> {
        (self)(T::from_context(&context))
    }
}

impl<F, T1, T2, M, B> TrainCommandHandler<B, (T1, T2), M> for F
where
    F: Fn(T1, T2) -> TrainResult<M>,
    T1: FromTrainCommandContext<B>,
    T2: FromTrainCommandContext<B>,
    M: Module<B>,
    B: Backend,
{
    fn call(self, context: TrainCommandContext<B>) -> TrainResult<M> {
        (self)(T1::from_context(&context), T2::from_context(&context))
    }
}

impl <F, T1, T2, T3, M, B> TrainCommandHandler<B, (T1, T2, T3), M> for F
where
    F: Fn(T1, T2, T3) -> TrainResult<M>,
    T1: FromTrainCommandContext<B>,
    T2: FromTrainCommandContext<B>,
    T3: FromTrainCommandContext<B>,
    M: Module<B>,
    B: Backend,
{
    fn call(self, context: TrainCommandContext<B>) -> TrainResult<M> {
        (self)(T1::from_context(&context), T2::from_context(&context), T3::from_context(&context))
    }
}