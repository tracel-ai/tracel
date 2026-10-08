//! Linking the experiments a job starts to the job.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tracel_experiment::{ExperimentLocation, ExperimentRun, Experiments};
use tracel_job::{JOB_NUM_ATTRIBUTE, ReportedExperiment};
use tracel_runtime::JobScope;

/// `experiments`, linked to the job the current thread runs, when it runs one, as [`link`] links
/// them; as they are otherwise. The link holds on any thread a run is started on.
pub fn linked(experiments: Experiments) -> Experiments {
    match RunningJob::current() {
        Some(job) => link(experiments, job),
        None => experiments,
    }
}

/// What runs once the job is asked to stop.
type StopAction = Box<dyn FnOnce() + Send>;

/// What a job gives the experiments started in it.
struct RunningJob {
    /// The number the job's launcher gave it.
    num: Option<String>,
    /// The input the job runs with, as JSON.
    input: Value,
    /// Registers an action to run once the job is asked to stop.
    on_stop: Box<dyn FnOnce(StopAction)>,
    /// Adds an experiment to the job's run report.
    report: Arc<dyn Fn(ReportedExperiment) + Send + Sync>,
}

impl RunningJob {
    /// The job the current thread runs, as its [`JobScope`] gives it, while it runs one.
    fn current() -> Option<Self> {
        let scope = JobScope::current()?;
        Some(Self {
            num: scope.num().map(str::to_string),
            input: scope.input().clone(),
            on_stop: Box::new({
                let scope = scope.clone();
                move |action: StopAction| scope.on_stop(action)
            }),
            report: Arc::new(move |experiment| scope.report_experiment(experiment)),
        })
    }
}

/// `experiments`, each started from which belongs to `job`: it records the job's number as its
/// `tracel.job_num` attribute and the job's input, unless it is `null`, as its arguments, it is
/// asked to stop once the job is, and no more start then, and it is reported as the job reports
/// experiments.
fn link(experiments: Experiments, job: RunningJob) -> Experiments {
    let cancel_token = experiments.cancel_token();
    (job.on_stop)(Box::new(move || cancel_token.cancel()));
    let attributes = job
        .num
        .map(|num| (JOB_NUM_ATTRIBUTE.to_string(), job_num(&num)));
    let experiments = match job.input {
        Value::Null => experiments,
        input => experiments.arguments(input),
    };
    let report = job.report;
    experiments
        .attributes(HashMap::from_iter(attributes))
        .on_start(move |run| report(reported(run)))
}

/// `run` as a job's run report lists it: its number, and its page or its directory.
fn reported(run: &ExperimentRun) -> ReportedExperiment {
    let (url, dir) = match run.location() {
        Some(ExperimentLocation::Url(url)) => (Some(url.clone()), None),
        Some(ExperimentLocation::Dir(dir)) => (None, Some(dir.clone())),
        None => (None, None),
    };
    ReportedExperiment {
        num: run.id().parse(),
        url,
        dir,
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
    use std::sync::Mutex;

    use serde_json::json;
    use tracel_experiment::ExperimentProvider;
    use tracel_experiment::error::ExperimentErrorKind;
    use tracel_experiment::local::LocalExperiments;

    use super::*;

    /// What a test keeps of what it is given.
    type Kept<T> = Arc<Mutex<Vec<T>>>;

    /// A job numbered `num`, running with `input`, the actions it runs once asked to stop, and the
    /// experiments it reports.
    fn job(
        num: Option<&str>,
        input: Value,
    ) -> (RunningJob, Kept<StopAction>, Kept<ReportedExperiment>) {
        let on_stop = Arc::new(Mutex::new(Vec::new()));
        let reported = Arc::new(Mutex::new(Vec::new()));
        let job = RunningJob {
            num: num.map(str::to_string),
            input,
            on_stop: Box::new({
                let on_stop = on_stop.clone();
                move |action| on_stop.lock().unwrap().push(action)
            }),
            report: Arc::new({
                let reported = reported.clone();
                move |experiment| reported.lock().unwrap().push(experiment)
            }),
        };
        (job, on_stop, reported)
    }

    fn offline(dir: &Path) -> Experiments {
        Experiments::new(Arc::new(LocalExperiments::new(dir)))
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
        let (job, on_stop, reported) = job(Some("12"), json!({"epochs": 2}));

        let experiments = link(offline(dir.path()), job);
        let run = experiments.start("train").unwrap();
        let actions: Vec<StopAction> = on_stop.lock().unwrap().drain(..).collect();
        for action in actions {
            action();
        }
        let stopped = experiments.start("train");
        run.finish().unwrap();

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
        assert!(
            events(dir.path(), 1)
                .iter()
                .any(|event| event["type"] == "log" && event["level"] == "warn"),
            "no stop warning"
        );
        assert!(matches!(
            stopped,
            Err(error) if error.kind == ExperimentErrorKind::Cancelled
        ));
        assert_eq!(
            *reported.lock().unwrap(),
            [ReportedExperiment {
                num: Some(1),
                url: None,
                dir: Some(runs.join("train/1")),
            }]
        );
    }

    #[test]
    fn an_experiment_started_in_a_job_without_a_number_or_input_records_neither() {
        let dir = tempfile::tempdir().unwrap();
        let (job, _, reported) = job(None, Value::Null);

        link(offline(dir.path()), job)
            .start("train")
            .unwrap()
            .finish()
            .unwrap();

        assert!(
            events(dir.path(), 1)
                .iter()
                .all(|event| event["type"] != "attribute" && event["type"] != "arguments")
        );
        assert_eq!(reported.lock().unwrap().len(), 1);
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
    fn an_experiment_is_reported_with_its_page_or_its_directory() {
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
            reported(&offline),
            ReportedExperiment {
                num: Some(1),
                url: None,
                dir: Some(dir.path().canonicalize().unwrap().join("mnist/1")),
            }
        );
        assert_eq!(
            reported(&on_the_console),
            ReportedExperiment {
                num: Some(2),
                url: Some(page.to_string()),
                dir: None,
            }
        );
    }
}
