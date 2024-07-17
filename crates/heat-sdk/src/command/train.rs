use burn::{config::Config, module::Module, tensor::backend::Backend};

use crate::client::HeatClient;

#[derive(Debug, Clone)]
pub struct DeviceVec<T>(pub Vec<T>);

#[derive(Debug, Clone)]
pub struct ConfigValue<T: Config>(pub T);


#[derive(Debug, Clone)]
pub struct TrainCommandContext<T> {
    client: HeatClient,
    devices: Vec<T>,
    config: String,
}

impl<T> TrainCommandContext<T> {
    pub fn new(client: HeatClient, devices: Vec<T>, config: String) -> Self {
        Self {
            client,
            devices,
            config,
        }
    }

    pub fn client(&mut self) -> &mut HeatClient {
        &mut self.client
    }

    pub fn devices(&mut self) -> &mut Vec<T> {
        &mut self.devices
    }

    pub fn config(&self) -> &str {
        &self.config
    }

    pub fn into_inner(self) -> (HeatClient, Vec<T>, String) {
        (self.client, self.devices, self.config)
    }
    
}

trait FromTrainCommandContext<T> {
    fn from_context(context: &TrainCommandContext<T>) -> Self;
}

impl<T> FromTrainCommandContext<T> for HeatClient {
    fn from_context(context: &TrainCommandContext<T>) -> Self {
        println!("Inferred usage of context.client");
        context.client.clone()
    }
}

impl<T: Clone> FromTrainCommandContext<T> for DeviceVec<T> {
    fn from_context(context: &TrainCommandContext<T>) -> Self {
        println!("Inferred usage of context.devices");
        DeviceVec(context.devices.clone())
    }
}

impl<T> IntoIterator for DeviceVec<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<T, C: Config> FromTrainCommandContext<T> for C  {
    fn from_context(context: &TrainCommandContext<T>) -> Self {
        println!("Inferred usage of context.config");
        C::load_binary(context.config.as_bytes()).expect("Config should be loaded")
    }
}

pub type TrainResult<M> = Result<M, ()>;

pub trait TrainCommandHandler<D, B: Backend, T, M: Module<B>> {
    fn call(self, context: TrainCommandContext<D>) -> TrainResult<M>;
}


impl<D, F, T, M, B> TrainCommandHandler<D, B, (T,), M> for F
where
    F: Fn(T) -> TrainResult<M>,
    T: FromTrainCommandContext<D>,
    M: Module<B>,
    B: Backend,
{
    fn call(self, context: TrainCommandContext<D>) -> TrainResult<M> {
        (self)(T::from_context(&context))
    }
}

impl<D, F, T1, T2, M, B> TrainCommandHandler<D, B, (T1, T2), M> for F
where
    F: Fn(T1, T2) -> TrainResult<M>,
    T1: FromTrainCommandContext<D>,
    T2: FromTrainCommandContext<D>,
    M: Module<B>,
    B: Backend,
{
    fn call(self, context: TrainCommandContext<D>) -> TrainResult<M> {
        (self)(T1::from_context(&context), T2::from_context(&context))
    }
}

impl <D, F, T1, T2, T3, M, B> TrainCommandHandler<D, B, (T1, T2, T3), M> for F
where
    F: Fn(T1, T2, T3) -> TrainResult<M>,
    T1: FromTrainCommandContext<D>,
    T2: FromTrainCommandContext<D>,
    T3: FromTrainCommandContext<D>,
    M: Module<B>,
    B: Backend,
{
    fn call(self, context: TrainCommandContext<D>) -> TrainResult<M> {
        (self)(T1::from_context(&context), T2::from_context(&context), T3::from_context(&context))
    }
}