use crate::{
    job::{Job, JobFunction},
    mapper::Mapper,
};
use std::collections::HashMap;

#[derive(Default)]
pub struct JobRegister {
    jobs: HashMap<String, JobFunction>,
}

impl JobRegister {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
        }
    }

    fn erase_job<J, I, O, F>(job: J, mapper: F) -> JobFunction
    where
        J: Job<I, O> + Send + Sync + 'static,
        F: Mapper<I> + Send + Sync + 'static,
        I: 'static,
        O: 'static,
    {
        Box::new(move |config_str: &str| {
            let input = mapper.map(config_str)?;
            job.execute(input).map(|_| ())?;
            Ok(())
        })
    }

    pub fn register<J, I, O, F>(mut self, job: J, mapper: F) -> Self
    where
        J: Job<I, O> + Send + Sync + 'static,
        F: Mapper<I> + Send + Sync + 'static,
        I: 'static,
        O: 'static,
    {
        let name = job.name().to_string();
        if self.jobs.contains_key(&name) {
            panic!("job '{}' is already registered", name);
        }
        let erased = Self::erase_job(job, mapper);
        self.jobs.insert(name, erased);
        self
    }

    pub fn job_names(&self) -> Vec<String> {
        self.jobs.keys().cloned().collect()
    }

    pub fn has_job(&self, name: &str) -> bool {
        self.jobs.contains_key(name)
    }

    pub fn dispatch(
        &self,
        job_name: &str,
        config: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let runner = self.jobs.get(job_name).ok_or_else(|| {
            format!(
                "unknown job '{}'. Available: {}",
                job_name,
                self.jobs.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        runner(config)
    }
}
