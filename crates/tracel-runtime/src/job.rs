use std::error::Error;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tracel_job::JobDefinition;

/// The error a job fails with.
pub type BoxError = Box<dyn Error + Send + Sync>;

/// A job: a function of its input, with what a launcher reads about it in its definition.
///
/// The function takes the job's input, decoded from JSON. The job completes when the function
/// returns `Ok`, and fails when it returns `Err` or panics. A function alone is a job with no
/// description, default input or input schema; `Job` adds them. A job may run more than once, and
/// on any thread, as its transport runs it: its input is decoded where the job is prepared and
/// sent to the thread that runs it.
pub struct Job<I> {
    run: Arc<dyn Fn(I) -> Result<(), BoxError> + Send + Sync>,
    description: Option<String>,
    default: Option<Value>,
    schema: Option<Value>,
}

impl<I> Job<I>
where
    I: DeserializeOwned + 'static,
{
    /// The job that runs `run` with its input.
    pub fn new<F, E>(run: F) -> Self
    where
        F: Fn(I) -> Result<(), E> + Send + Sync + 'static,
        E: Into<BoxError>,
    {
        Self {
            run: Arc::new(move |input| run(input).map_err(Into::into)),
            description: None,
            default: None,
            schema: None,
        }
    }

    /// Describes what the job does, as its definition and its `--help` give it.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Runs the job with its input merged onto `default`, which its definition gives as its
    /// example input, and from which its flags take the defaults they show.
    ///
    /// With no input, the job runs with `default`. An input is merged onto it with JSON merge
    /// patch (RFC 7386): its fields replace the default's, and a `null` field removes one.
    ///
    /// # Panics
    ///
    /// When `default` does not serialize to JSON.
    pub fn with_default(mut self, default: I) -> Self
    where
        I: Serialize,
    {
        self.default =
            Some(serde_json::to_value(default).expect("default input must serialize to JSON"));
        self
    }

    /// Gives the JSON Schema of `I` as the job's input schema, from which its command line has a
    /// flag per field of the input, with its type, allowed values, requiredness and help
    /// (requires the `schema` feature).
    #[cfg(feature = "schema")]
    pub fn with_schema(mut self) -> Self
    where
        I: schemars::JsonSchema,
    {
        self.schema = Some(schemars::schema_for!(I).to_value());
        self
    }
}

/// What [`Jobs::job`](crate::Jobs::job) registers: a [`Job`], or a function of the job's input.
pub trait IntoJob<I> {
    /// The job.
    fn into_job(self) -> Job<I>;
}

impl<I> IntoJob<I> for Job<I> {
    fn into_job(self) -> Job<I> {
        self
    }
}

impl<I, F, E> IntoJob<I> for F
where
    I: DeserializeOwned + 'static,
    F: Fn(I) -> Result<(), E> + Send + Sync + 'static,
    E: Into<BoxError>,
{
    fn into_job(self) -> Job<I> {
        Job::new(self)
    }
}

/// A job registered under a name, whatever its input type.
pub struct Registered {
    definition: JobDefinition,
    default: Option<Value>,
    decode: Decode,
}

/// Decodes a job's input, and gives the job ready to run with it.
type Decode = Box<dyn Fn(&Value) -> Result<Ready, serde_json::Error> + Send + Sync>;

/// A job given its input, ready to run on the thread it is sent to.
pub type Ready = Box<dyn FnOnce() -> Result<(), BoxError> + Send>;

/// A job given its input, ready to run, and that input.
pub struct Prepared {
    /// The input the job runs with, as JSON: the input given, merged onto the job's default.
    pub input: Value,
    /// The job, given the input decoded.
    pub run: Ready,
}

impl Registered {
    /// `job`, registered as `name`.
    pub fn new<I>(name: String, job: Job<I>) -> Self
    where
        I: DeserializeOwned + Send + 'static,
    {
        let Job {
            run,
            description,
            default,
            schema,
        } = job;
        Self {
            definition: JobDefinition {
                name,
                description,
                input_schema: schema,
                input_example: default.clone(),
            },
            default,
            decode: Box::new(move |input| {
                let input = I::deserialize(input)?;
                let run = run.clone();
                Ok(Box::new(move || run(input)) as Ready)
            }),
        }
    }

    /// The job's definition.
    pub fn definition(&self) -> &JobDefinition {
        &self.definition
    }

    /// The job, ready to run with `input` merged onto its default, when it has one.
    /// `Value::Null` stands for no input.
    ///
    /// Fails without running anything when the input does not decode as the job's input.
    pub fn prepare(&self, input: Value) -> Result<Prepared, serde_json::Error> {
        let input = resolve(self.default.clone(), input);
        let run = (self.decode)(&input)?;
        Ok(Prepared { input, run })
    }
}

/// `input` merged onto `default`: `default` when there is no input, and `input` as given when
/// there is no default.
fn resolve(default: Option<Value>, input: Value) -> Value {
    match default {
        Some(default) if input.is_null() => default,
        Some(mut default) => {
            json_patch::merge(&mut default, &input);
            default
        }
        None => input,
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::Mutex;

    use rstest::rstest;
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
    struct Config {
        epochs: u32,
        optimizer: Optimizer,
        tag: Option<String>,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
    struct Optimizer {
        lr: f64,
        weight_decay: f64,
    }

    fn default_config() -> Config {
        Config {
            epochs: 10,
            optimizer: Optimizer {
                lr: 0.001,
                weight_decay: 0.0,
            },
            tag: Some("baseline".to_string()),
        }
    }

    fn recording<I>() -> (Job<I>, Arc<Mutex<Option<I>>>)
    where
        I: DeserializeOwned + Send + 'static,
    {
        let ran_with = Arc::new(Mutex::new(None));
        let job = Job::new({
            let ran_with = ran_with.clone();
            move |input| {
                *ran_with.lock().unwrap() = Some(input);
                Ok::<_, BoxError>(())
            }
        });
        (job, ran_with)
    }

    fn run<I>(job: Job<I>, input: Value) -> Result<(), BoxError>
    where
        I: DeserializeOwned + Send + 'static,
    {
        (Registered::new("train".to_string(), job)
            .prepare(input)?
            .run)()
    }

    #[test]
    fn a_job_is_defined_by_its_name_description_and_default() {
        let (name, description) = ("train", "Train the model");
        let (job, _) = recording::<Config>();
        let job = job
            .with_description(description)
            .with_default(default_config());

        assert_eq!(
            Registered::new(name.to_string(), job).definition(),
            &JobDefinition {
                name: name.to_string(),
                description: Some(description.to_string()),
                input_schema: None,
                input_example: Some(serde_json::to_value(default_config()).unwrap()),
            }
        );
    }

    #[test]
    fn no_input_runs_the_job_with_its_default() {
        let (job, ran_with) = recording();

        run(job.with_default(default_config()), Value::Null).unwrap();

        assert_eq!(ran_with.lock().unwrap().take(), Some(default_config()));
    }

    #[test]
    fn an_input_is_merge_patched_onto_the_default() {
        let (job, ran_with) = recording();

        run(
            job.with_default(default_config()),
            json!({"epochs": 2, "optimizer": {"lr": 0.1}, "tag": null}),
        )
        .unwrap();

        assert_eq!(
            ran_with.lock().unwrap().take(),
            Some(Config {
                epochs: 2,
                optimizer: Optimizer {
                    lr: 0.1,
                    ..default_config().optimizer
                },
                tag: None,
            })
        );
    }

    #[rstest]
    #[case::a_field_of_the_wrong_type(json!({"epochs": "ten"}))]
    #[case::not_an_object(json!("ten"))]
    fn an_input_that_does_not_decode_is_refused(#[case] input: Value) {
        let (job, _) = recording::<Config>();

        let prepared =
            Registered::new("train".to_string(), job.with_default(default_config())).prepare(input);

        assert!(prepared.is_err());
    }

    #[test]
    fn a_registered_job_runs_each_time_it_is_prepared() {
        let (job, ran_with) = recording::<u32>();
        let registered = Registered::new("train".to_string(), job);
        let epochs = [1, 2];

        let ran = epochs.map(|epochs| {
            (registered.prepare(json!(epochs)).unwrap().run)().unwrap();
            ran_with.lock().unwrap().take()
        });

        assert_eq!(ran, epochs.map(Some));
    }

    #[test]
    fn a_job_fails_with_the_error_it_returns() {
        let job = Job::new(|_: ()| Err::<(), _>(io::Error::other("disk full")));

        let failed = run(job, Value::Null).unwrap_err();

        assert!(failed.is::<io::Error>());
    }

    #[cfg(feature = "schema")]
    #[test]
    fn with_schema_gives_the_input_types_schema() {
        let (job, _) = recording::<Config>();

        let registered = Registered::new("train".to_string(), job.with_schema());

        assert_eq!(
            registered.definition().input_schema,
            Some(schemars::schema_for!(Config).to_value())
        );
    }
}
