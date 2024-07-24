use strum::Display;

#[derive(Debug, Display)]
pub enum TrainingError {
    UnknownError(String),
}

impl From<()> for TrainingError {
    fn from(_: ()) -> Self {
        TrainingError::UnknownError("Unknown training error".to_string())
    }
}

impl std::error::Error for TrainingError {}
