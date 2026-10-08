//! [`IntoJob`] for experiment jobs.

use serde_json::Value;
use tracel_experiment::{ExperimentJob, ExperimentLocation, ExperimentRun};
use tracel_job::{JobDefinition, ReportedExperiment};

use crate::job::{BoxError, IntoJob, Job, PreparedJob};
use crate::mapper::Mapper;

impl<I, O, M> IntoJob<M> for ExperimentJob<I, O>
where
    I: Send + 'static,
    O: 'static,
    M: Mapper<I> + 'static,
{
    fn into_job(self, mapper: M) -> Box<dyn Job> {
        Box::new(Experiment {
            definition: JobDefinition {
                name: self.name().to_string(),
                description: self.description().map(str::to_string),
                input_schema: mapper.schema(),
                input_example: mapper.example(),
            },
            job: self,
            mapper,
        })
    }
}

/// An experiment job: one decoded input, run to completion. Its output is not returned.
struct Experiment<I, O, M> {
    definition: JobDefinition,
    job: ExperimentJob<I, O>,
    mapper: M,
}

impl<I, O, M> Job for Experiment<I, O, M>
where
    I: Send + 'static,
    O: 'static,
    M: Mapper<I>,
{
    fn definition(&self) -> &JobDefinition {
        &self.definition
    }

    fn prepare(&self, input: Value) -> Result<PreparedJob, BoxError> {
        // The experiment records the input it runs with: the JSON the mapper resolved.
        let arguments = self.mapper.resolve(input);
        let input = self.mapper.decode(arguments.clone())?;
        let job = self.job.clone();
        Ok(PreparedJob::new(move |context| {
            job.run_with(input, arguments, context.cancel_token().clone(), |run| {
                context.reporter().report(reported(run));
            })
            .map(|_| ())
        }))
    }
}

/// `run` as a job's run report links it: its number, and its page or its directory.
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use tracel_experiment::local::LocalExperiments;
    use tracel_experiment::{CancelToken, ExperimentProvider, Experiments};

    use super::*;
    use crate::mapper::JsonMapper;
    use crate::test_support::NeverRuns;
    use crate::{ExperimentReporter, JobContext};

    #[test]
    fn an_experiment_is_defined_by_its_job_and_mapper() {
        let job = Experiments::new(Arc::new(NeverRuns))
            .create("train", |_run: &ExperimentRun, _epochs: u32| Ok(()))
            .with_description("Train the model")
            .into_job(JsonMapper::with_default(10u32));

        assert_eq!(
            job.definition(),
            &JobDefinition {
                name: "train".to_string(),
                description: Some("Train the model".to_string()),
                input_schema: None,
                input_example: Some(json!(10)),
            }
        );
    }

    #[test]
    fn an_experiment_takes_an_input_that_decodes() {
        let job = Experiments::new(Arc::new(NeverRuns))
            .create("train", |_run: &ExperimentRun, _epochs: u32| Ok(()))
            .into_job(JsonMapper::with_default(10u32));

        assert!(job.prepare(json!(3)).is_ok());
        assert!(job.prepare(Value::Null).is_ok());
        assert!(job.prepare(json!("ten")).is_err());
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn an_experiment_records_the_input_its_mapper_resolved_as_its_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let job = Experiments::new(Arc::new(LocalExperiments::new(dir.path())))
            .create("train", |_run: &ExperimentRun, _config: Value| Ok(()))
            .into_job(JsonMapper::with_default(
                json!({"epochs": 10, "optimizer": {"lr": 0.001}}),
            ));

        job.prepare(json!({"epochs": 2}))
            .unwrap()
            .run(JobContext::default())
            .unwrap();

        let events = std::fs::read_to_string(dir.path().join("train/1/events.jsonl")).unwrap();
        let arguments: Vec<Value> = events
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|event| event["type"] == "arguments")
            .collect();
        assert_eq!(
            arguments,
            [json!({
                "type": "arguments",
                "value": {"epochs": 2, "optimizer": {"lr": 0.001}}
            })]
        );
    }

    #[test]
    fn cancelling_the_token_given_asks_the_experiment_to_stop() {
        let dir = tempfile::tempdir().unwrap();
        let cancel_token = CancelToken::new();
        let job = Experiments::new(Arc::new(LocalExperiments::new(dir.path())))
            .create("train", {
                let cancel_token = cancel_token.clone();
                move |run: &ExperimentRun, _epochs: u32| {
                    cancel_token.cancel();
                    assert!(run.cancel_token().is_cancelled());
                    Ok(())
                }
            })
            .into_job(JsonMapper::with_default(10u32));

        job.prepare(Value::Null)
            .unwrap()
            .run(JobContext::new(cancel_token))
            .unwrap();

        let status = read_json(&dir.path().join("train/1/status.json"));
        assert_eq!(status["status"], "completed");
    }

    #[test]
    fn an_experiment_hands_the_experiment_it_creates_to_the_reporter_before_it_runs() {
        let dir = tempfile::tempdir().unwrap();
        let reported = Arc::new(Mutex::new(Vec::new()));
        let reporter = ExperimentReporter::new({
            let reported = reported.clone();
            move |experiment| reported.lock().unwrap().push(experiment)
        });
        let job = Experiments::new(Arc::new(LocalExperiments::new(dir.path())))
            .create("train", {
                let reported = reported.clone();
                move |_run: &ExperimentRun, _epochs: u32| {
                    assert_eq!(reported.lock().unwrap().len(), 1);
                    Ok(())
                }
            })
            .into_job(JsonMapper::with_default(10u32));

        job.prepare(Value::Null)
            .unwrap()
            .run(JobContext::default().with_reporter(reporter))
            .unwrap();

        assert_eq!(
            *reported.lock().unwrap(),
            [ReportedExperiment {
                num: Some(1),
                url: None,
                dir: Some(dir.path().canonicalize().unwrap().join("train/1")),
            }]
        );
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
