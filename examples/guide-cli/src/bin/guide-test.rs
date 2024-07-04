use guide_cli::inference::heat_inference_main;
use guide_cli::training::heat_training_main;

fn main() {
    println!("Running bin.");
    let model = heat_training_main();
    if let Ok(model) = model {
        println!("Model trained successfully.");
        heat_inference_main(model);
    } else {
        println!("Model training failed.");
    }
}
