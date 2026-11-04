//! Linking the experiments a job starts to the job.

use std::collections::HashMap;

use serde_json::Value;
use tracel_experiment::{ExperimentLocation, ExperimentRun, Experiments};
use tracel_runtime::JobScope;

/// The experiment attribute that links an experiment to the job that started it: the job's
/// number, as a number when it is one.
const JOB_NUM_ATTRIBUTE: &str = "tracel.job_num";

/// `experiments`, linked to the job the current thread runs, when it runs one, as [`link`] links
/// them, each run started from them printing where it records to stderr; as they are otherwise.
/// The link holds on any thread a run is started on.
pub fn linked(experiments: Experiments) -> Experiments {
    match RunningJob::current() {
        Some(job) => link(experiments, job, |line| eprintln!("{line}")),
        None => experiments,
    }
}

/// What a job gives the experiments started in it.
struct RunningJob {
    /// The number the job's launcher gave it.
    num: Option<String>,
    /// The input the job runs with, as JSON.
    input: Value,
}

impl RunningJob {
    /// The job the current thread runs, as its [`JobScope`] gives it, while it runs one.
    fn current() -> Option<Self> {
        let scope = JobScope::current()?;
        Some(Self {
            num: scope.num().map(str::to_string),
            input: scope.input().clone(),
        })
    }
}

/// `experiments`, each started from which belongs to `job`: it records the job's number as its
/// `tracel.job_num` attribute and the job's input, unless it is `null`, as its arguments, and it
/// gives `announce` the line saying where it records as it starts.
fn link(
    experiments: Experiments,
    job: RunningJob,
    announce: impl Fn(&str) + Send + Sync + 'static,
) -> Experiments {
    let attributes = job
        .num
        .map(|num| (JOB_NUM_ATTRIBUTE.to_string(), job_num(&num)));
    let experiments = match job.input {
        Value::Null => experiments,
        input => experiments.arguments(input),
    };
    experiments
        .attributes(HashMap::from_iter(attributes))
        .on_start(move |run| announce(&recording(run)))
}

/// The line saying where `run` records: its number and its page on the console, or its directory
/// offline.
fn recording(run: &ExperimentRun) -> String {
    match run.location() {
        Some(ExperimentLocation::Url(url)) => format!("Recording experiment {}: {url}", run.id()),
        Some(ExperimentLocation::Dir(dir)) => format!("Recording offline in {}", dir.display()),
        None => format!("Recording experiment {}", run.id()),
    }
}

/// The attribute value of the job number `num`: a number when it is one, its text otherwise.
fn job_num(num: &str) -> Value {
    match num.parse::<u64>() {
        Ok(num) => Value::from(num),
        Err(_) => Value::from(num),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use tracel_experiment::ExperimentProvider;
    use tracel_experiment::local::LocalExperiments;

    use super::*;

    /// What a test keeps of what it is given.
    type Kept<T> = Arc<Mutex<Vec<T>>>;

    /// A job numbered `num`, running with `input`.
    fn job(num: Option<&str>, input: Value) -> RunningJob {
        RunningJob {
            num: num.map(str::to_string),
            input,
        }
    }

    fn offline(dir: &Path) -> Experiments {
        Experiments::new(Arc::new(LocalExperiments::new(dir)))
    }

    /// `experiments` linked to `job`, and the lines its runs announce.
    fn linked_to(experiments: Experiments, job: RunningJob) -> (Experiments, Kept<String>) {
        let announced = Arc::new(Mutex::new(Vec::new()));
        let experiments = link(experiments, job, {
            let announced = announced.clone();
            move |line| announced.lock().unwrap().push(line.to_string())
        });
        (experiments, announced)
    }

    /// The lines of the events file of run `num` of `train`, under `dir`.
    fn events(dir: &Path, num: u64) -> Vec<Value> {
        std::fs::read_to_string(dir.join(format!("train/{num}/events.jsonl")))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn an_experiment_started_in_a_job_belongs_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let job = job(Some("12"), json!({"epochs": 2}));

        let (experiments, announced) = linked_to(offline(dir.path()), job);
        experiments.start("train").unwrap().finish().unwrap();

        let runs = dir.path().canonicalize().unwrap();
        assert!(events(dir.path(), 1).contains(&json!({
            "type": "attribute",
            "key": "tracel.job_num",
            "value": 12
        })));
        assert!(events(dir.path(), 1).contains(&json!({
            "type": "arguments",
            "value": {"epochs": 2}
        })));
        assert_eq!(
            *announced.lock().unwrap(),
            [format!(
                "Recording offline in {}",
                runs.join("train/1").display()
            )]
        );
    }

    #[test]
    fn an_experiment_started_in_a_job_without_a_number_or_input_records_neither() {
        let dir = tempfile::tempdir().unwrap();
        let job = job(None, Value::Null);
        let (experiments, announced) = linked_to(offline(dir.path()), job);

        experiments.start("train").unwrap().finish().unwrap();

        assert!(
            events(dir.path(), 1)
                .iter()
                .all(|event| event["type"] != "attribute" && event["type"] != "arguments")
        );
        assert_eq!(announced.lock().unwrap().len(), 1);
    }

    #[test]
    fn outside_a_job_experiments_are_not_linked() {
        let dir = tempfile::tempdir().unwrap();

        linked(offline(dir.path()))
            .start("train")
            .unwrap()
            .finish()
            .unwrap();

        assert!(
            events(dir.path(), 1)
                .iter()
                .all(|event| event["type"] != "attribute" && event["type"] != "arguments")
        );
    }

    #[test]
    fn a_job_number_is_recorded_as_a_number_when_it_is_one() {
        assert_eq!(job_num("42"), json!(42));
        assert_eq!(job_num("job-42"), json!("job-42"));
    }

    #[test]
    fn an_experiment_announces_its_page_or_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let experiments = LocalExperiments::new(dir.path());
        let page = "https://console.tracel.ai/users/me/projects/mnist/experiments/2";

        let offline = experiments
            .create_experiment("mnist".to_string(), HashMap::new())
            .unwrap();
        let on_the_console = experiments
            .create_experiment("mnist".to_string(), HashMap::new())
            .unwrap()
            .with_location(ExperimentLocation::Url(page.to_string()));

        assert_eq!(
            recording(&offline),
            format!(
                "Recording offline in {}",
                dir.path().canonicalize().unwrap().join("mnist/1").display()
            )
        );
        assert_eq!(
            recording(&on_the_console),
            format!("Recording experiment 2: {page}")
        );
    }
}
