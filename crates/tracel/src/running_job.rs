//! Linking the experiments a job starts to the job.

use std::collections::HashMap;

use serde_json::Value;
use tracel_experiment::{ExperimentLocation, ExperimentRun, Experiments};
use tracel_runtime::JobScope;

/// The experiment attribute that links an experiment to the job that started it: the job's
/// number, as a number when it is one.
const JOB_NUM_ATTRIBUTE: &str = "tracel.job_num";

/// `experiments`, linked to the job the current thread runs, when it runs one; as they are
/// otherwise.
///
/// Each experiment started from linked experiments, on any thread, belongs to the job: it records
/// the job's number as its `tracel.job_num` attribute and the job's input, unless it is `null`, as
/// its arguments, and it prints where it records to stderr as it starts.
pub fn linked(experiments: Experiments) -> Experiments {
    let Some(job) = JobScope::current() else {
        return experiments;
    };
    let attributes = job
        .num()
        .map(|num| (JOB_NUM_ATTRIBUTE.to_string(), job_num(num)));
    let experiments = match job.input() {
        Value::Null => experiments,
        input => experiments.arguments(input.clone()),
    };
    experiments
        .attributes(HashMap::from_iter(attributes))
        .on_start(|run| eprintln!("{}", recording(run)))
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
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use rstest::{fixture, rstest};
    use serde_json::json;
    use tracel_experiment::error::ExperimentError;
    use tracel_experiment::reader::{
        ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact,
    };
    use tracel_experiment::session::{BundleFn, Event, ExperimentCompletion, ExperimentSession};
    use tracel_experiment::{ArtifactKind, CancelToken, ExperimentId, ExperimentProvider};
    use tracel_runtime::{BoxError, Execution, Jobs};

    use super::*;

    const ID: &str = "7";
    const PAGE: &str = "https://console.tracel.ai/users/alice/projects/mnist/experiments/7";

    #[derive(Debug, Default, PartialEq)]
    struct Recorded {
        attributes: HashMap<String, Value>,
        arguments: Option<Value>,
    }

    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Recorded>>);

    impl Recorder {
        fn experiments(&self) -> Experiments {
            Experiments::new(Arc::new(self.clone()))
        }

        fn recorded(&self) -> Recorded {
            std::mem::take(&mut self.0.lock().unwrap())
        }
    }

    impl ExperimentProvider for Recorder {
        fn create_experiment(
            &self,
            _: String,
            attributes: HashMap<String, Value>,
        ) -> Result<ExperimentRun, ExperimentError> {
            self.0.lock().unwrap().attributes = attributes;
            Ok(ExperimentRun::new(
                ID,
                self.clone(),
                self.clone(),
                CancelToken::new(),
            ))
        }
    }

    impl ExperimentSession for Recorder {
        fn record_event(&self, event: Event) -> Result<(), ExperimentError> {
            if let Event::Args(arguments) = event {
                self.0.lock().unwrap().arguments = Some(arguments);
            }
            Ok(())
        }

        fn save_artifact(
            &self,
            _: &str,
            _: ArtifactKind,
            _: Box<BundleFn>,
        ) -> Result<(), ExperimentError> {
            Ok(())
        }

        fn finish(&self, _: ExperimentCompletion) -> Result<(), ExperimentError> {
            Ok(())
        }
    }

    impl ExperimentArtifactReader for Recorder {
        fn load_artifact_raw(
            &self,
            _: ExperimentId,
            _: &str,
        ) -> Result<LoadedArtifact, ExperimentReaderError> {
            Err(ExperimentReaderError::new("a recorder keeps no artifacts"))
        }
    }

    type Start = fn(Experiments) -> Result<(), BoxError>;

    fn start(experiments: Experiments) -> Result<(), BoxError> {
        experiments.start("train")?.finish()?;
        Ok(())
    }

    fn start_on_another_thread(experiments: Experiments) -> Result<(), BoxError> {
        std::thread::spawn(move || start(experiments))
            .join()
            .unwrap()
    }

    fn recorded_in_a_job(execution: Execution, input: Value, start: Start) -> Recorded {
        let recorder = Recorder::default();
        let train = {
            let recorder = recorder.clone();
            move |_: Value| start(linked(recorder.experiments()))
        };
        Jobs::new()
            .job("train", train)
            .prepare("train", input)
            .unwrap()
            .run(execution)
            .into_result()
            .unwrap();
        recorder.recorded()
    }

    #[fixture]
    fn input() -> Value {
        json!({"epochs": 2})
    }

    #[rstest]
    #[case::on_the_jobs_thread(start)]
    #[case::on_a_thread_the_job_spawns(start_on_another_thread)]
    fn an_experiment_started_in_a_job_belongs_to_it(#[case] start: Start, input: Value) {
        let num = "12";

        let recorded = recorded_in_a_job(Execution::new().with_num(num), input.clone(), start);

        assert_eq!(
            recorded,
            Recorded {
                attributes: HashMap::from([(JOB_NUM_ATTRIBUTE.to_string(), job_num(num))]),
                arguments: Some(input),
            }
        );
    }

    #[test]
    fn an_experiment_started_in_a_job_without_a_number_or_input_records_neither() {
        assert_eq!(
            recorded_in_a_job(Execution::new(), Value::Null, start),
            Recorded::default()
        );
    }

    #[test]
    fn outside_a_job_experiments_are_not_linked() {
        let recorder = Recorder::default();

        start(linked(recorder.experiments())).unwrap();

        assert_eq!(recorder.recorded(), Recorded::default());
    }

    #[test]
    fn a_job_number_is_recorded_as_a_number_when_it_is_one() {
        assert_eq!(job_num("42"), json!(42));
        assert_eq!(job_num("job-42"), json!("job-42"));
    }

    #[rstest]
    #[case::on_the_console(Some(ExperimentLocation::Url(PAGE.to_string())), PAGE)]
    #[case::offline(
        Some(ExperimentLocation::Dir(PathBuf::from("runs/mnist/1"))),
        "runs/mnist/1"
    )]
    #[case::without_a_location(None, ID)]
    fn an_experiment_announces_where_it_records(
        #[case] location: Option<ExperimentLocation>,
        #[case] named: &str,
    ) {
        let run = ExperimentRun::new(
            ID,
            Recorder::default(),
            Recorder::default(),
            CancelToken::new(),
        );
        let run = match location {
            Some(location) => run.with_location(location),
            None => run,
        };

        assert!(recording(&run).contains(named));
    }
}
