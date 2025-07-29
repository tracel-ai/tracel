use burn::prelude::Backend;
use burn_central_client::experiment::ExperimentConfig;
use derive_more::{Deref, From};

#[derive(From, Deref)]
pub struct Cfg<T: ExperimentConfig>(pub T);
#[derive(Clone, Debug, Deref, From)]
pub struct MultiDevice<B: Backend>(pub Vec<B::Device>);
#[derive(Clone, From, Deref)]
pub struct Model<M>(pub M);
