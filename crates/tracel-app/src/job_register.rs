use crate::{
    job::{Job, RunFn, ValidateFn},
    mapper::Mapper,
};
use std::{any::Any, collections::HashMap};

struct JobEntry {
    validate: ValidateFn,
    run: RunFn,
}

#[derive(Default)]
pub struct JobRegister {
    jobs: HashMap<String, JobEntry>,
}

impl JobRegister {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
        }
    }

    fn erase_job<J, I, O, M>(job: J, mapper: M) -> JobEntry
    where
        J: Job<I, O> + Send + Sync + 'static,
        M: Mapper<I> + Send + Sync + 'static,
        I: Send + 'static,
        O: 'static,
    {
        let validate: ValidateFn = Box::new(move |config_str: &str| {
            let input = mapper.map(config_str)?;
            Ok(Box::new(input) as Box<dyn Any + Send>)
        });

        let run: RunFn = Box::new(move |input: Box<dyn Any + Send>| {
            let input = *input
                .downcast::<I>()
                .expect("type mismatch in job dispatch");
            job.execute(input).map(|_| ())
        });

        JobEntry { validate, run }
    }

    pub fn register<J, I, O, F>(mut self, job: J, mapper: F) -> Self
    where
        J: Job<I, O> + Send + Sync + 'static,
        F: Mapper<I> + Send + Sync + 'static,
        I: Send + 'static,
        O: 'static,
    {
        let name = job.name().to_string();
        if self.jobs.contains_key(&name) {
            panic!("job '{}' is already registered", name);
        }
        let entry = Self::erase_job(job, mapper);
        self.jobs.insert(name, entry);
        self
    }

    pub fn job_names(&self) -> Vec<String> {
        self.jobs.keys().cloned().collect()
    }

    pub fn has_job(&self, name: &str) -> bool {
        self.jobs.contains_key(name)
    }

    pub fn validate(
        &self,
        job_name: &str,
        config: &str,
    ) -> Result<Box<dyn Any + Send>, Box<dyn std::error::Error + Send + Sync>> {
        let entry = self.jobs.get(job_name).ok_or_else(|| {
            format!(
                "unknown job '{}'. Available: {}",
                job_name,
                self.jobs.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        (entry.validate)(config)
    }

    pub fn run(
        &self,
        job_name: &str,
        input: Box<dyn Any + Send>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let entry = self.jobs.get(job_name).ok_or_else(|| {
            format!(
                "unknown job '{}'. Available: {}",
                job_name,
                self.jobs.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        (entry.run)(input)
    }

    pub fn dispatch(
        &self,
        job_name: &str,
        config: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let input = self.validate(job_name, config)?;
        self.run(job_name, input)
    }
}
