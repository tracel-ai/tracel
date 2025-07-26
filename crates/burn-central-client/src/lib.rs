pub mod api;
mod client;
pub mod command;
pub mod credentials;
pub mod error;

pub mod schemas;

pub mod log;
pub mod metrics;
pub mod record;

pub mod experiment;
mod websocket;

pub use crate::client::*;

mod api_test {
    use burn::prelude::Backend;

    pub trait ExperimentWorkflow<B: Backend> {
        fn name(&self) -> &'static str;
        fn run(&self, device: B::Device);
    }

    pub struct BurnCentralApp<B: Backend> {
        workflows: Vec<Box<dyn ExperimentWorkflow<B>>>,
    }

    impl<B: Backend> BurnCentralApp<B> {
        pub fn new() -> Self {
            Self { workflows: vec![] }
        }

        pub fn add<W: ExperimentWorkflow<B> + 'static>(mut self, w: W) -> Self {
            self.workflows.push(Box::new(w));
            self
        }

        pub fn build(self, device: B::Device) -> BuiltTrainingApp<B> {
            BuiltTrainingApp {
                device,
                workflows: self.workflows,
            }
        }
    }

    pub struct BuiltTrainingApp<B: Backend> {
        device: B::Device,
        workflows: Vec<Box<dyn ExperimentWorkflow<B>>>,
    }

    impl<B: Backend> BuiltTrainingApp<B> {
        pub fn run_by_name(&self, name: &str, device: B::Device) {
            for wf in &self.workflows {
                if wf.name() == name {
                    wf.run(device);
                    return;
                }
            }
            eprintln!("Workflow not found: {name}");
        }

        pub fn list(&self) -> impl Iterator<Item = &'static str> {
            self.workflows.iter().map(|w| w.name())
        }
    }
}
