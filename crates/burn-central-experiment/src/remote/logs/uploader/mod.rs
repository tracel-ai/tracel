use burn_central_artifact::bundle::InMemoryBundleSources;

use crate::remote::logs::LogStoreError;

pub mod central;
pub mod station;

pub trait LogUploader {
    fn upload(&mut self, bundle: InMemoryBundleSources) -> Result<(), LogStoreError>;
}
type BoxedLogUploader = Box<dyn LogUploader + Send>;
