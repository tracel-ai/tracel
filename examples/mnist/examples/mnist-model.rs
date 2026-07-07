#![recursion_limit = "256"]

use mnist::model::MnistModelArtifact;
use tracel::Connection;
use tracel::Context;

fn main() -> anyhow::Result<()> {
    const MODEL_NAME: &str = "mnist";
    const MODEL_VERSION: u32 = 1;

    let model: MnistModelArtifact = Context::new(Connection::Cloud)?
        .models()
        .unwrap()
        .load(MODEL_NAME, MODEL_VERSION, &())
        .unwrap();

    println!("{model:#?}");

    Ok(())
}
