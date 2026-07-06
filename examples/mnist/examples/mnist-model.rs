#![recursion_limit = "256"]

use std::path::PathBuf;

use tracel::Connection;
use tracel::Context;
use tracel::artifact::bundle::FsBundle;

fn main() -> anyhow::Result<()> {
    const MODEL_NAME: &str = "mnist";
    const MODEL_VERSION: u32 = 1;

    let model_registry = Context::new(Connection::Cloud)?.model_registry().unwrap();

    let model = model_registry.get(MODEL_NAME)?;
    let model_version = model_registry.version(MODEL_NAME, MODEL_VERSION)?;
    println!("{model:?}");
    println!("{model_version:?}");

    let downloads_dir = PathBuf::from(std::env::var("HOME")?).join("Downloads");
    let mut sink = FsBundle::create(downloads_dir)?;
    model_registry.download_to(&model.name, model_version.version, &mut sink)?;

    Ok(())
}
