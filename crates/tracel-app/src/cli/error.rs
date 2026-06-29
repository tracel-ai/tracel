use crate::job_register::JobRegisterError;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("no job name given and no default registered")]
    MissingDefault,

    #[error(transparent)]
    JobRegister(#[from] JobRegisterError),
}
