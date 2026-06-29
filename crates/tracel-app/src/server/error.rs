use crate::job_register::JobRegisterError;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("server error: {0}")]
    IoError(#[from] std::io::Error),

    #[error(transparent)]
    JobRegister(#[from] JobRegisterError),
}
