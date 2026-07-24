use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::error::{ExperimentError, ExperimentErrorKind};
use crate::integration::tracing::try_init_tracing_subscriber;
use crate::{ExperimentRun, ExperimentRunHandleExt};

pub trait ExperimentProvider: Send + Sync + 'static {
    fn create_experiment(
        &self,
        name: String,
        attributes: HashMap<String, Value>,
    ) -> Result<ExperimentRun, ExperimentError>;
}

pub trait ExperimentFn<I, O>: Send + Sync {
    fn call(&self, run: &ExperimentRun, input: I) -> Result<O, Box<dyn Error + Send + Sync>>;
}

impl<I, O, F> ExperimentFn<I, O> for F
where
    F: Fn(&ExperimentRun, I) -> Result<O, Box<dyn Error + Send + Sync>> + Send + Sync,
{
    fn call(&self, run: &ExperimentRun, input: I) -> Result<O, Box<dyn Error + Send + Sync>> {
        (self)(run, input)
    }
}

pub struct ExperimentModule {
    provider: Arc<dyn ExperimentProvider>,
}

impl ExperimentModule {
    // TODO: Add settings here (e.g., an ExperimentModule builder).
    pub fn new(provider: Arc<dyn ExperimentProvider>) -> Self {
        Self { provider }
    }

    pub fn create<I, O>(
        &self,
        name: &str,
        f: impl ExperimentFn<I, O> + 'static,
    ) -> ExperimentJob<I, O> {
        ExperimentJob::new(self.provider.clone(), name.to_string(), f)
    }
}

pub struct ExperimentJob<I, O> {
    provider: Arc<dyn ExperimentProvider>,
    name: String,
    attributes: HashMap<String, Value>,
    f: Arc<dyn ExperimentFn<I, O>>,
}

impl<I, O> Clone for ExperimentJob<I, O> {
    fn clone(&self) -> Self {
        Self {
            provider: self.provider.clone(),
            name: self.name.clone(),
            attributes: self.attributes.clone(),
            f: self.f.clone(),
        }
    }
}

impl<I, O> ExperimentJob<I, O> {
    fn new<F>(provider: Arc<dyn ExperimentProvider>, name: String, f: F) -> Self
    where
        F: ExperimentFn<I, O> + 'static,
    {
        Self {
            provider,
            name,
            attributes: HashMap::new(),
            f: Arc::new(f),
        }
    }

    #[doc(hidden)]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn attribute(
        mut self,
        key: impl Into<String>,
        value: impl Serialize,
    ) -> Result<Self, ExperimentError> {
        let value = serde_json::to_value(value).map_err(|e| {
            ExperimentError::with_source(
                ExperimentErrorKind::Internal,
                "Failed to serialize experiment attribute",
                e,
            )
        })?;

        self.attributes.insert(key.into(), value);
        Ok(self)
    }

    pub fn attributes(mut self, attrs: HashMap<String, Value>) -> Self {
        self.attributes.extend(attrs);
        self
    }

    pub fn run(&self, input: I) -> Result<O, Box<dyn std::error::Error + Send + Sync>> {
        let _ = try_init_tracing_subscriber();

        let experiment = self
            .provider
            .create_experiment(self.name.clone(), self.attributes.clone())?;
        let handle = experiment.handle();
        let result = handle.in_scope(|| self.f.call(&experiment, input));

        match result {
            Ok(output) => {
                if experiment.cancel_token().is_cancelled() {
                    // Dropping without an explicit completion finalizes the run as cancelled.
                    drop(experiment);
                } else {
                    experiment.finish()?;
                }
                Ok(output)
            }
            Err(e) if experiment.cancel_token().is_cancelled() => {
                drop(experiment);
                Err(e)
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = experiment.fail(msg);
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::reader::{ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact};
    use crate::session::{BundleFn, ExperimentCompletion, ExperimentSession};
    use crate::{ArtifactKind, CancelToken, ExperimentId};

    #[derive(Default)]
    struct MockSession {
        completions: Mutex<Vec<ExperimentCompletion>>,
    }

    impl ExperimentSession for MockSession {
        fn record_event(&self, _event: crate::session::Event) -> Result<(), ExperimentError> {
            Ok(())
        }

        fn save_artifact(
            &self,
            _name: &str,
            _kind: ArtifactKind,
            _artifact: Box<BundleFn>,
        ) -> Result<(), ExperimentError> {
            Ok(())
        }

        fn finish(&self, completion: ExperimentCompletion) -> Result<(), ExperimentError> {
            self.completions.lock().unwrap().push(completion);
            Ok(())
        }
    }

    #[derive(Default)]
    struct NoopExperimentDataReader;

    impl ExperimentArtifactReader for NoopExperimentDataReader {
        fn load_artifact_raw(
            &self,
            _experiment_id: ExperimentId,
            _name: &str,
        ) -> Result<LoadedArtifact, ExperimentReaderError> {
            Err(ExperimentReaderError::new("Artifact not found"))
        }
    }

    struct MockProvider {
        session: Arc<MockSession>,
    }

    impl ExperimentProvider for MockProvider {
        fn create_experiment(
            &self,
            _name: String,
            _attributes: HashMap<String, Value>,
        ) -> Result<ExperimentRun, ExperimentError> {
            Ok(ExperimentRun::new(
                "test/experiment/1",
                self.session.clone(),
                NoopExperimentDataReader,
                CancelToken::new(),
            ))
        }
    }

    fn module(session: Arc<MockSession>) -> ExperimentModule {
        ExperimentModule::new(Arc::new(MockProvider { session }))
    }

    #[test]
    fn given_run_when_completing_then_finalizes_with_success() {
        let session = Arc::new(MockSession::default());
        let job = module(session.clone()).create("job", |_run: &ExperimentRun, _input: ()| Ok(()));

        job.run(()).unwrap();

        let completions = session.completions.lock().unwrap();
        assert_eq!(completions.as_slice(), &[ExperimentCompletion::Success]);
    }

    #[test]
    fn given_run_cancelled_through_control_when_running_then_complete_cancelled() {
        let session = Arc::new(MockSession::default());
        let job = module(session.clone()).create("job", |run: &ExperimentRun, _input: ()| {
            // A remote backend would cancel the run's own control plane; simulate that here.
            run.cancel_token().cancel();
            Ok(())
        });

        job.run(()).unwrap();

        let completions = session.completions.lock().unwrap();
        assert_eq!(completions.as_slice(), &[ExperimentCompletion::Cancelled]);
    }

    #[test]
    fn given_run_error_when_cancel_started_then_complete_cancelled() {
        let session = Arc::new(MockSession::default());
        let job = module(session.clone()).create("job", |run: &ExperimentRun, _input: ()| {
            run.cancel_token().cancel();
            Err::<(), _>("interrupted".into())
        });

        let result = job.run(());

        assert!(result.is_err());
        let completions = session.completions.lock().unwrap();
        assert_eq!(completions.as_slice(), &[ExperimentCompletion::Cancelled]);
    }

    #[test]
    fn given_run_error_when_running_then_complete_failed() {
        let session = Arc::new(MockSession::default());
        let job = module(session.clone()).create("job", |_run: &ExperimentRun, _input: ()| {
            Err::<(), _>("boom".into())
        });

        let result = job.run(());

        assert!(result.is_err());
        let completions = session.completions.lock().unwrap();
        assert_eq!(
            completions.as_slice(),
            &[ExperimentCompletion::Failed("boom".to_string())]
        );
    }
}
