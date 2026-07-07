#![recursion_limit = "256"]

use mnist::model::MnistModelArtifact;
use tracel::Connection;
use tracel::Context;

fn main() -> anyhow::Result<()> {
    const MODEL_NAME: &str = "mnist";
    const MODEL_VERSION: u32 = 1;

    let model_registry = Context::new(Connection::Cloud)?.model_registry().unwrap();
    let model: MnistModelArtifact = model_registry.load(MODEL_NAME, MODEL_VERSION, &()).unwrap();

    println!("{model:#?}");

    Ok(())
}
