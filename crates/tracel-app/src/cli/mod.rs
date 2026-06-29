mod error;

pub use error::CliError;

use crate::{job::Job, job_register::JobRegister, mapper::Mapper};
use clap::Parser;
use std::error::Error;

#[derive(Parser)]
#[command(about = "Run a registered job")]
struct Args {
    job: Option<String>,
    config: Option<String>,
}

struct DefaultJob {
    runner: Box<dyn FnOnce() -> Result<(), Box<dyn Error + Send + Sync>>>,
}

#[derive(Default)]
pub struct Cli {
    register: JobRegister,
    default: Option<DefaultJob>,
}

impl Cli {
    pub fn new() -> Self {
        Self {
            register: JobRegister::new(),
            default: None,
        }
    }

    pub fn register<J, I, O, F>(mut self, job: J, mapper: F) -> Self
    where
        J: Job<I, O> + Send + Sync + 'static,
        F: Mapper<I> + Send + Sync + 'static,
        I: Send + 'static,
        O: 'static,
    {
        self.register = self.register.register(job, mapper);
        self
    }

    pub fn default_job<J, I, O>(mut self, job: J, config: I) -> Self
    where
        J: Job<I, O> + Send + Sync + 'static,
        I: Send + 'static,
        O: 'static,
    {
        self.default = Some(DefaultJob {
            runner: Box::new(move || job.execute(config).map(|_| ())),
        });
        self
    }

    pub fn run(self) -> Result<(), CliError> {
        let args = Args::parse();
        self.dispatch(args.job, args.config)
    }

    fn dispatch(self, job: Option<String>, config: Option<String>) -> Result<(), CliError> {
        match job {
            Some(job_name) => {
                if !self.register.has_job(&job_name) {
                    return Err(CliError::UnknownJob {
                        name: job_name,
                        available: self.register.job_names(),
                    });
                }
                let config_str = config.unwrap_or_default();
                self.register
                    .dispatch(&job_name, &config_str)
                    .map_err(CliError::JobError)
            }
            None => {
                let d = self.default.ok_or(CliError::MissingDefault)?;
                (d.runner)().map_err(CliError::JobError)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::Job;
    use crate::mapper::Mapper;
    use std::error::Error;

    struct FakeJob {
        name: &'static str,
        should_fail: bool,
    }

    impl FakeJob {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                should_fail: false,
            }
        }

        fn failing(name: &'static str) -> Self {
            Self {
                name,
                should_fail: true,
            }
        }
    }

    impl Job<String, ()> for FakeJob {
        fn name(&self) -> &str {
            self.name
        }

        fn execute(&self, _input: String) -> Result<(), Box<dyn Error + Send + Sync>> {
            if self.should_fail {
                Err("job execution failed".into())
            } else {
                Ok(())
            }
        }
    }

    struct FakeMapper {
        should_fail: bool,
    }

    impl FakeMapper {
        fn new() -> Self {
            Self { should_fail: false }
        }

        fn failing() -> Self {
            Self { should_fail: true }
        }
    }

    impl Mapper<String> for FakeMapper {
        fn map(&self, raw: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
            if self.should_fail {
                Err("mapper failed".into())
            } else {
                Ok(raw.to_string())
            }
        }
    }

    #[test]
    fn dispatch_named_job_ok() {
        let cli = Cli::new().register(FakeJob::new("train"), FakeMapper::new());

        let result = cli.dispatch(Some("train".into()), Some("{}".into()));

        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_named_job_unknown() {
        let cli = Cli::new().register(FakeJob::new("train"), FakeMapper::new());

        let result = cli.dispatch(Some("infer".into()), Some("{}".into()));

        assert!(matches!(result, Err(CliError::UnknownJob { .. })));
    }

    #[test]
    fn dispatch_named_job_config_none_defaults_config_to_empty_string() {
        let cli = Cli::new().register(FakeJob::new("train"), FakeMapper::new());

        let result = cli.dispatch(Some("train".into()), None);

        assert!(result.is_ok());
    }

    #[test]
    fn mapper_error_is_wrapped_in_job_error() {
        let cli = Cli::new().register(FakeJob::new("train"), FakeMapper::failing());

        let result = cli.dispatch(Some("train".into()), Some("{}".into()));

        assert!(matches!(result, Err(CliError::JobError(_))));
    }

    #[test]
    fn job_error_is_wrapped_in_job_error() {
        let cli = Cli::new().register(FakeJob::failing("train"), FakeMapper::new());

        let result = cli.dispatch(Some("train".into()), Some("{}".into()));

        assert!(matches!(result, Err(CliError::JobError(_))));
    }

    #[test]
    fn dispatch_default_job_ok() {
        let cli = Cli::new().default_job(FakeJob::new("default"), "config".to_string());

        let result = cli.dispatch(None, None);

        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_no_job_no_default() {
        let cli = Cli::new();

        let result = cli.dispatch(None, None);

        assert!(matches!(result, Err(CliError::MissingDefault)));
    }

    #[test]
    fn dispatch_default_job_fails() {
        let cli = Cli::new().default_job(FakeJob::failing("default"), "config".to_string());

        let result = cli.dispatch(None, None);

        assert!(matches!(result, Err(CliError::JobError(_))));
    }

    #[test]
    fn dispatch_multiple_jobs_picks_correct_one() {
        let cli = Cli::new()
            .register(FakeJob::new("train"), FakeMapper::new())
            .register(FakeJob::new("infer"), FakeMapper::new());

        let result = cli.dispatch(Some("infer".into()), Some("{}".into()));

        assert!(result.is_ok());
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn register_duplicate_job_panics() {
        Cli::new()
            .register(FakeJob::new("train"), FakeMapper::new())
            .register(FakeJob::new("train"), FakeMapper::new());
    }
}
