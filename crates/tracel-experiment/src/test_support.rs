use std::sync::{Arc, Mutex};

use crate::activity::ActivityEvent;
use crate::error::ExperimentError;
use crate::reader::{ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact};
use tracel_artifact::bundle::FsBundle;
use tracel_task::Job;

use crate::session::{Event, ExperimentCompletion, ExperimentSession};
use crate::{ArtifactKind, ExperimentId, ExperimentRun, ExperimentRunControl};

#[derive(Default)]
pub(crate) struct MockSession {
    pub(crate) events: Mutex<Vec<Event>>,
    pub(crate) completions: Mutex<Vec<ExperimentCompletion>>,
    pub(crate) flushes: Mutex<usize>,
}

impl MockSession {
    pub(crate) fn activity_events(&self) -> Vec<ActivityEvent> {
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
}

impl ExperimentSession for MockSession {
    fn record_event(&self, event: Event) -> Result<(), ExperimentError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }

    fn flush(&self) -> Job<(), ExperimentError> {
        *self.flushes.lock().unwrap() += 1;
        Job::ready(())
    }

    fn save_artifact(
        &self,
        _name: String,
        _kind: ArtifactKind,
        _bundle: FsBundle,
    ) -> Result<(), ExperimentError> {
        Ok(())
    }

    fn finish(&self, completion: ExperimentCompletion) -> Job<(), ExperimentError> {
        self.completions.lock().unwrap().push(completion);
        Job::ready(())
    }
}

#[derive(Default)]
struct NoopExperimentDataReader;

impl ExperimentArtifactReader for NoopExperimentDataReader {
    fn load_artifact_raw(
        &self,
        _experiment_id: ExperimentId,
        _name: String,
    ) -> Job<LoadedArtifact, ExperimentReaderError> {
        Job::failed(ExperimentReaderError::new("Artifact not found"))
    }
}

pub(crate) fn create_run(session: Arc<MockSession>) -> ExperimentRun {
    create_run_with_id("test/experiment/1", session)
}

pub(crate) fn create_run_with_id(
    id: impl Into<ExperimentId>,
    session: Arc<MockSession>,
) -> ExperimentRun {
    ExperimentRun::new(
        id,
        session,
        NoopExperimentDataReader,
        crate::CancelToken::default(),
    )
}

pub(crate) fn create_run_with_control(
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
