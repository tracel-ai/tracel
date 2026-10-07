//! [`IntoJob`] for the capability jobs: experiments and inferences.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tracel_experiment::ExperimentJob;
use tracel_inference::{InferenceJob, OutputWriter, OutputWriterError};

use crate::job::{BoxError, IntoJob, Job, JobDefinition, JobInput, JobKind, PreparedJob};
use crate::mapper::Mapper;

fn definition<I>(
    name: &str,
    kind: JobKind,
    description: Option<&str>,
    mapper: &dyn Mapper<I>,
) -> JobDefinition {
    JobDefinition {
        name: name.to_string(),
        kind,
        description: description.map(str::to_string),
        input_schema: mapper.schema(),
        input_example: mapper.example(),
    }
}

impl<I, O, M> IntoJob<M> for ExperimentJob<I, O>
where
    I: Send + 'static,
    O: 'static,
    M: Mapper<I> + 'static,
{
    fn into_job(self, mapper: M) -> Box<dyn Job> {
        Box::new(Experiment {
            definition: definition(
                self.name(),
                JobKind::Experiment,
                self.description(),
                &mapper,
            ),
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

    fn prepare(&self, input: JobInput) -> Result<PreparedJob, BoxError> {
        let JobInput::Document(input) = input else {
            return Err(format!(
                "experiment '{}' takes one JSON document, not a stream",
                self.definition.name
            )
            .into());
        };
        // The experiment records the input it runs with: the JSON the mapper resolved.
        let arguments = self.mapper.resolve(input);
        let input = self.mapper.decode(arguments.clone())?;
        let job = self.job.clone();
        Ok(PreparedJob::new(move |_output, cancel_token| {
            job.run_with(input, arguments, cancel_token).map(|_| ())
        }))
    }
}

impl<I, O, M> IntoJob<M> for InferenceJob<I, O>
where
    I: Send + 'static,
    O: Serialize + Send + Sync + 'static,
    M: Mapper<I> + 'static,
{
    fn into_job(self, mapper: M) -> Box<dyn Job> {
        Box::new(Inference {
            definition: definition(self.name(), JobKind::Inference, self.description(), &mapper),
            job: self,
            mapper: Arc::new(mapper),
        })
    }
}

/// An inference job: each decoded input is answered with outputs, sent on as JSON.
struct Inference<I, O, M> {
    definition: JobDefinition,
    job: InferenceJob<I, O>,
    mapper: Arc<M>,
}

impl<I, O, M> Job for Inference<I, O, M>
where
    I: Send + 'static,
    O: Serialize + Send + Sync + 'static,
    M: Mapper<I> + 'static,
{
    fn definition(&self) -> &JobDefinition {
        &self.definition
    }

    fn prepare(&self, input: JobInput) -> Result<PreparedJob, BoxError> {
        let job = self.job.clone();
        match input {
            JobInput::Document(input) => {
                let input = self.mapper.map(input)?;
                Ok(PreparedJob::new(move |output, _cancel_token| {
                    let output = JsonOutput(Arc::from(output));
                    Ok(job.run(std::iter::once(input), output)?)
                }))
            }
            JobInput::Stream(inputs) => {
                let mapper = self.mapper.clone();
                Ok(PreparedJob::new(move |output, cancel_token| {
                    let output: Arc<dyn OutputWriter<Value> + Send + Sync> = Arc::from(output);
                    let errors = output.clone();
                    // An input that does not decode is reported as an error and ends the stream,
                    // and cancelling ends it before the next input.
                    let inputs = inputs
                        .take_while(move |_| !cancel_token.is_cancelled())
                        .map_while(move |input| match mapper.map(input) {
                            Ok(input) => Some(input),
                            Err(error) => {
                                let _ = errors.error(error);
                                None
                            }
                        });
                    Ok(job.run(inputs, JsonOutput(output))?)
                }))
            }
        }
    }
}

/// Sends an inference's typed outputs on to a job's output as JSON.
struct JsonOutput(Arc<dyn OutputWriter<Value> + Send + Sync>);

impl<O: Serialize> OutputWriter<O> for JsonOutput {
    fn write(&self, output: O) -> Result<(), OutputWriterError> {
        match serde_json::to_value(output) {
            Ok(output) => self.0.write(output),
            Err(error) => self.0.error(Box::new(error)),
        }
    }

    fn error(&self, error: BoxError) -> Result<(), OutputWriterError> {
        self.0.error(error)
    }

    fn finish(&self, duration: Duration) {
        self.0.finish(duration);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;

    use serde::Deserialize;
    use serde_json::json;
    use tracel_experiment::local::LocalExperiments;
    use tracel_experiment::{CancelToken, ExperimentRun, Experiments};
    use tracel_inference::{
        InferenceInput, InferenceModule, InferenceOutput, InferenceSession, NoopInferenceProvider,
    };

    use super::*;
    use crate::DiscardOutput;
    use crate::mapper::JsonMapper;
    use crate::test_support::NeverRuns;

    #[derive(Debug, Serialize, Deserialize)]
    struct Prompt {
        text: String,
    }

    #[derive(Default, Clone)]
    struct Collected(Arc<Mutex<Vec<Result<Value, String>>>>);

    impl OutputWriter<Value> for Collected {
        fn write(&self, output: Value) -> Result<(), OutputWriterError> {
            self.0.lock().unwrap().push(Ok(output));
            Ok(())
        }

        fn error(&self, error: BoxError) -> Result<(), OutputWriterError> {
            self.0.lock().unwrap().push(Err(error.to_string()));
            Ok(())
        }

        fn finish(&self, _duration: Duration) {}
    }

    fn words() -> Box<dyn Job> {
        InferenceModule::new(Arc::new(NoopInferenceProvider::new()))
            .create(
                "words",
                |_session: &InferenceSession,
                 input: InferenceInput<Prompt>,
                 output: InferenceOutput<String>| {
                    for prompt in input {
                        for word in prompt.text.split_whitespace() {
                            let _ = output.write(word.to_string());
                        }
                    }
                },
            )
            .with_description("Split prompts into words")
            .into_job(JsonMapper::<Prompt>::new())
    }

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
                kind: JobKind::Experiment,
                description: Some("Train the model".to_string()),
                input_schema: None,
                input_example: Some(json!(10)),
            }
        );
    }

    #[test]
    fn an_experiment_rejects_an_input_that_does_not_decode_or_a_stream() {
        let job = Experiments::new(Arc::new(NeverRuns))
            .create("train", |_run: &ExperimentRun, _epochs: u32| Ok(()))
            .into_job(JsonMapper::with_default(10u32));

        assert!(job.prepare(JobInput::Document(json!("ten"))).is_err());
        assert!(
            job.prepare(JobInput::Stream(Box::new(std::iter::empty())))
                .is_err()
        );
        assert!(job.prepare(JobInput::Document(json!(3))).is_ok());
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

        job.prepare(JobInput::Document(json!({"epochs": 2})))
            .unwrap()
            .run(DiscardOutput, CancelToken::new())
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
    fn cancelling_the_token_given_cancels_the_experiment() {
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

        job.prepare(JobInput::Document(Value::Null))
            .unwrap()
            .run(DiscardOutput, cancel_token)
            .unwrap();

        let status = read_json(&dir.path().join("train/1/status.json"));
        assert_eq!(status["status"], "cancelled");
    }

    #[test]
    fn an_inference_sends_its_outputs_as_json() {
        let output = Collected::default();

        words()
            .prepare(JobInput::Document(json!({"text": "hello streaming world"})))
            .unwrap()
            .run(output.clone(), CancelToken::new())
            .unwrap();

        assert_eq!(
            *output.0.lock().unwrap(),
            vec![
                Ok(json!("hello")),
                Ok(json!("streaming")),
                Ok(json!("world"))
            ]
        );
    }

    #[test]
    fn an_inference_stream_ends_at_the_first_input_that_does_not_decode() {
        let output = Collected::default();
        let inputs = vec![
            json!({"text": "one two"}),
            json!({"prompt": "three"}),
            json!({"text": "four"}),
        ];

        words()
            .prepare(JobInput::Stream(Box::new(inputs.into_iter())))
            .unwrap()
            .run(output.clone(), CancelToken::new())
            .unwrap();

        let output = output.0.lock().unwrap();
        assert_eq!(output[..2], [Ok(json!("one")), Ok(json!("two"))]);
        assert!(matches!(&output[2], Err(error) if error.contains("text")));
        assert_eq!(output.len(), 3);
    }
}
