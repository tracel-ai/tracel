fn main() {
    println!("Running bin.");
    let model = guide_cli::training::heat_training_main();

    if let Ok(model) = model {
        println!("Model trained successfully.");
        guide_cli::inference::heat_inference_main(model);
    } else {
        println!("Model training failed.");
    }
}
