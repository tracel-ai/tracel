use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct HeatSdkRunMetadata {
    pub options: Options,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Options {
    pub backend: String,
}

pub fn load_metadata(path: &str) -> Result<HeatSdkRunMetadata, String> {
    let metadata = toml::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    Ok(metadata)
}
