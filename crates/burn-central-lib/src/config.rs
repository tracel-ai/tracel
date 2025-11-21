#[derive(Debug, Clone)]
pub struct Config {
    pub api_endpoint: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            api_endpoint: String::from("https://heat.tracel.ai/api/"),
        }
    }
}
