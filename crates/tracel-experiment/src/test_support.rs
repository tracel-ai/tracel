use std::sync::{Arc, Mutex};

use rstest::fixture;

use crate::activity::{ActivityEvent, ActivityId, ActivitySpec};
use crate::error::ExperimentError;
use crate::reader::{ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact};
use crate::session::{BundleFn, Event, ExperimentCompletion, ExperimentSession};
use crate::{ArtifactKind, ExperimentId, ExperimentRun, ExperimentRunControl, LogRecord};

#[derive(Default)]
pub struct MockSession {
    pub events: Mutex<Vec<Event>>,
    pub completions: Mutex<Vec<ExperimentCompletion>>,
    pub flushes: Mutex<usize>,
}

impl MockSession {
    pub fn activity_events(&self) -> Vec<ActivityEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                Event::Activity(event) => Some(event.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn started_activities(&self) -> Vec<ActivitySpec> {
        self.activity_events()
            .into_iter()
            .filter_map(|event| match event {
                ActivityEvent::Started { activity } => Some(activity),
                _ => None,
            })
            .collect()
    }

    pub fn logs(&self) -> Vec<LogRecord> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                Event::Log { record, .. } => Some(record.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn log_activities(&self) -> Vec<Option<ActivityId>> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                Event::Log { activity, .. } => Some(*activity),
                _ => None,
            })
            .collect()
    }

    pub fn log_messages(&self) -> Vec<String> {
        self.logs()
            .into_iter()
            .map(|record| record.message)
            .collect()
    }
}

impl ExperimentSession for MockSession {
    fn record_event(&self, event: Event) -> Result<(), ExperimentError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }

    fn flush(&self) -> Result<(), ExperimentError> {
        *self.flushes.lock().unwrap() += 1;
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

pub fn create_run(session: Arc<MockSession>) -> ExperimentRun {
    create_run_with_id("test/experiment/1", session)
}

pub fn create_run_with_id(id: impl Into<ExperimentId>, session: Arc<MockSession>) -> ExperimentRun {
    ExperimentRun::new(
        id,
        session,
        NoopExperimentDataReader,
        crate::CancelToken::default(),
    )
}

pub fn create_run_with_control(
    session: Arc<MockSession>,
    control: ExperimentRunControl,
) -> ExperimentRun {
    ExperimentRun::new_with_control(
        "test/experiment/1",
        session,
        NoopExperimentDataReader,
        control,
    )
}

pub struct MockRun {
    pub run: ExperimentRun,
    pub session: Arc<MockSession>,
}

#[fixture]
pub fn mock_run() -> MockRun {
    let session = Arc::new(MockSession::default());
    MockRun {
        run: create_run(session.clone()),
        session,
    }
}
