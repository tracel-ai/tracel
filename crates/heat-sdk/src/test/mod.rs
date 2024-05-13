use std::collections::HashMap;

pub fn test() -> String {
    let mut body = HashMap::new();
    body.insert("file_path", "testy");

    let client = reqwest::blocking::Client::new();
    let res = client.post("http://localhost:8080/checkpoints")
        .json(&body)
        .send()
        .expect("Failed to send request").text();

    res.unwrap()
}
