use std::collections::BTreeMap;
use std::sync::Arc;

use tracel_job::JobDefinition;

use crate::job::Job;

/// The jobs a runner can run, by name.
///
/// Each runner keeps one, filled by its `register` methods.
#[derive(Default)]
pub struct JobRegistry {
    jobs: BTreeMap<String, Arc<dyn Job>>,
}

impl JobRegistry {
    /// A registry with no jobs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `job` under the name its definition gives.
    ///
    /// # Panics
    ///
    /// When a job with that name is already registered.
    pub fn add(&mut self, job: Box<dyn Job>) {
        let name = job.definition().name.clone();
        if self.jobs.contains_key(&name) {
            panic!("job '{name}' is already registered");
        }
        self.jobs.insert(name, Arc::from(job));
    }

    /// The job named `name`.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Job>> {
        self.jobs.get(name).cloned()
    }

    /// The definitions of the registered jobs, ordered by name.
    pub fn definitions(&self) -> impl Iterator<Item = &JobDefinition> {
        self.jobs.values().map(|job| job.definition())
    }

    /// The names of the registered jobs, in order.
    pub fn names(&self) -> Vec<String> {
        self.jobs.keys().cloned().collect()
    }

    /// Whether no job is registered.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::job::{BoxError, JobInput, PreparedJob};

    struct Named(JobDefinition);

    impl Job for Named {
        fn definition(&self) -> &JobDefinition {
            &self.0
        }

        fn prepare(&self, _input: JobInput) -> Result<PreparedJob, BoxError> {
            Ok(PreparedJob::new(|_output, _context| Ok(())))
        }
    }

    fn job(name: &str) -> Box<dyn Job> {
        Box::new(Named(JobDefinition {
            name: name.to_string(),
            description: None,
            input_schema: None,
            input_example: Some(json!({"name": name})),
        }))
    }

    #[test]
    fn jobs_are_listed_by_name() {
        let mut registry = JobRegistry::new();
        registry.add(job("train"));
        registry.add(job("evaluate"));

        let names: Vec<&str> = registry
            .definitions()
            .map(|definition| definition.name.as_str())
            .collect();

        assert_eq!(names, ["evaluate", "train"]);
        assert_eq!(registry.names(), ["evaluate", "train"]);
        assert_eq!(
            registry.get("train").unwrap().definition().input_example,
            Some(json!({"name": "train"}))
        );
        assert!(registry.get("infer").is_none());
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn a_name_registers_once() {
        let mut registry = JobRegistry::new();
        registry.add(job("train"));
        registry.add(job("train"));
    }
}
