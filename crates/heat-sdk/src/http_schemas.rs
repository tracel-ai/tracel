use serde::Deserialize;

#[derive(Deserialize)]
pub struct URLSchema {
    pub url: String,
}
