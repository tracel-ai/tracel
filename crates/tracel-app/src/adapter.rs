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
        let input = self.mapper.map(&input)?;
        let job = self.job.clone();
        Ok(PreparedJob::new(move |_output| job.run(input).map(|_| ())))
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
                let input = self.mapper.map(&input)?;
                Ok(PreparedJob::new(move |output| {
                    let output = JsonOutput(Arc::from(output));
                    Ok(job.run(std::iter::once(input), output)?)
                }))
            }
            JobInput::Stream(inputs) => {
                let mapper = self.mapper.clone();
                Ok(PreparedJob::new(move |output| {
                    let output: Arc<dyn OutputWriter<Value> + Send + Sync> = Arc::from(output);
                    let errors = output.clone();
                    // An input that does not decode is reported as an error and ends the stream.
                    let inputs = inputs.map_while(move |input| match mapper.map(&input) {
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
    use std::sync::Mutex;

    use serde::Deserialize;
    use serde_json::json;
    use tracel_experiment::{ExperimentRun, Experiments};
    use tracel_inference::{
        InferenceInput, InferenceModule, InferenceOutput, InferenceSession, NoopInferenceProvider,
    };

    use super::*;
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

    #[test]
    fn an_inference_sends_its_outputs_as_json() {
        let output = Collected::default();

        words()
            .prepare(JobInput::Document(json!({"text": "hello streaming world"})))
            .unwrap()
            .run(output.clone())
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
            .run(output.clone())
            .unwrap();

        let output = output.0.lock().unwrap();
        assert_eq!(output[..2], [Ok(json!("one")), Ok(json!("two"))]);
        assert!(matches!(&output[2], Err(error) if error.contains("text")));
        assert_eq!(output.len(), 3);
    }
}
