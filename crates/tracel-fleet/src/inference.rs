use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use arc_swap::ArcSwapOption;
use tracel_artifact::bundle::FsBundle;
use tracel_inference::{
    Inference, InferenceId, InferenceInput, InferenceOutput, InferenceSession, OutputWriter,
};

use crate::FleetDeviceSession;
use crate::telemetry::{InferenceMetadata, MetricsSink};

#[derive(Debug, thiserror::Error)]
pub enum FleetManagedInferenceError {
    #[error("inference '{name}' failed to initialize: {message}")]
    FactoryFailed { name: String, message: String },
}

pub trait FleetManagedFactory<I>: Send + Sync {
    fn build(&self, model_source: FsBundle, runtime_config: serde_json::Value)
    -> Result<I, String>;
}

impl<F, I> FleetManagedFactory<I> for F
where
    F: Fn(FsBundle, serde_json::Value) -> Result<I, String> + Send + Sync,
{
    fn build(
        &self,
        model_source: FsBundle,
        runtime_config: serde_json::Value,
    ) -> Result<I, String> {
        self(model_source, runtime_config)
    }
}

struct ActiveInference<I> {
    inference: I,
    model_version: String,
}

/// Inference wrapper that bootstraps features like fleet registration and telemetry on top of a typed inference implementation.
pub struct FleetManagedInference<I> {
    inference_name: String,
    fleet_session: RwLock<FleetDeviceSession>,
    factory: Box<dyn FleetManagedFactory<I>>,
    active: ArcSwapOption<ActiveInference<I>>,
    reconcile_gate: Mutex<()>,
    last_sync_at: Mutex<Option<Instant>>,
    sync_interval: Duration,
}

impl<I> FleetManagedInference<I>
where
    I: Inference,
{
    pub fn init(
        inference_name: impl Into<String>,
        fleet_session: FleetDeviceSession,
        factory: Box<dyn FleetManagedFactory<I>>,
    ) -> Result<Self, FleetManagedInferenceError> {
        let inference = Self {
            inference_name: inference_name.into(),
            fleet_session: RwLock::new(fleet_session),
            factory,
            active: ArcSwapOption::empty(),
            reconcile_gate: Mutex::new(()),
            last_sync_at: Mutex::new(None),
            sync_interval: Duration::from_secs(10),
        };
        inference.ensure_ready()?;
        Ok(inference)
    }

    fn maybe_sync_and_rollout(&self) -> Result<(), FleetManagedInferenceError> {
        if self.active().is_some() && !self.should_sync_now() {
            return Ok(());
        }

        let _guard = self.reconcile_gate.lock().unwrap();
        if self.active().is_some() && !self.should_sync_now() {
            return Ok(());
        }

        let (fleet_version, model_source, config) = {
            let mut session = self.fleet_session.write().unwrap();

            match session.sync_for_reconcile() {
                Ok(()) => {
                    let fleet_version = normalized_model_version(session.active_model_version_id());
                    let model_source = session.model_source().map_err(|err| {
                        FleetManagedInferenceError::FactoryFailed {
                            name: self.inference_name.clone(),
                            message: format!("fleet model source failed: {err}"),
                        }
                    })?;

                    let config = session.runtime_config();

                    (fleet_version, model_source, config.clone())
                }
                Err(sync_err) => {
                    self.mark_sync_now();

                    if self.active().is_some() {
                        tracing::warn!(
                            err = %sync_err,
                            "fleet sync failed, keeping current active model"
                        );
                        return Ok(());
                    }

                    tracing::warn!(
                        err = %sync_err,
                         "fleet sync failed and no active model, trying local cache"
                    );

                    let fleet_version = normalized_model_version(session.active_model_version_id());
                    let model_source = session.model_source().map_err(|cache_err| {
                        FleetManagedInferenceError::FactoryFailed {
                            name: self.inference_name.clone(),
                            message: format!(
                                "fleet sync failed and no usable local cache: sync={sync_err}; cache={cache_err}"
                            ),
                        }
                    })?;
                    let config = session.runtime_config();

                    (fleet_version, model_source, config.clone())
                }
            }
        };

        self.mark_sync_now();

        let active = self.active();
        if active.as_ref().map(|a| &a.model_version) == Some(&fleet_version) {
            tracing::debug!(
                version = &fleet_version,
                "fleet model version is same as active, skipping rollout"
            );
            return Ok(());
        }

        let built = self
            .factory
            .build(model_source, config)
            .map_err(|message| FleetManagedInferenceError::FactoryFailed {
                name: self.inference_name.clone(),
                message,
            })?;

        self.active.store(Some(Arc::new(ActiveInference {
            inference: built,
            model_version: fleet_version,
        })));

        Ok(())
    }

    fn ensure_ready(&self) -> Result<(), FleetManagedInferenceError> {
        self.maybe_sync_and_rollout()?;
        if self.active().is_none() {
            return Err(FleetManagedInferenceError::FactoryFailed {
                name: self.inference_name.clone(),
                message: "no active model after bootstrap".to_string(),
            });
        }
        Ok(())
    }

    fn should_sync_now(&self) -> bool {
        let last_sync_at = self.last_sync_at.lock().unwrap();
        match *last_sync_at {
            Some(instant) => instant.elapsed() >= self.sync_interval,
            None => true,
        }
    }

    fn mark_sync_now(&self) {
        let mut last_sync_at = self.last_sync_at.lock().unwrap();
        *last_sync_at = Some(Instant::now());
    }

    fn active(&self) -> Option<Arc<ActiveInference<I>>> {
        self.active.load_full()
    }

    fn current_fleet_key(&self) -> String {
        self.fleet_session.read().unwrap().fleet_key().to_string()
    }

    fn metadata(&self, fleet_key: String, active: &ActiveInference<I>) -> InferenceMetadata {
        InferenceMetadata::new(
            fleet_key,
            self.inference_name.clone(),
            "unknown",
            active.model_version.clone(),
        )
    }

    /// Drive the managed model for one request, building a session that ships telemetry to the
    /// process-global `metrics` recorder tagged with fleet metadata.
    ///
    /// This is fleet's provider-free entry point: it owns the session, so no `InferenceProvider` or
    /// `InferenceJob` is involved. Per-request stats and any metrics the model records are scoped
    /// with `fleet_key`/`inference_name`/`model_name`/`model_version`.
    pub fn run<It, W>(
        &self,
        request_id: impl Into<InferenceId>,
        input: It,
        output: W,
    ) -> Result<(), FleetManagedInferenceError>
    where
        It: IntoIterator<Item = I::Input>,
        It::IntoIter: Send + 'static,
        W: OutputWriter<I::Output> + 'static,
    {
        self.maybe_sync_and_rollout()?;
        let active = self
            .active()
            .ok_or_else(|| FleetManagedInferenceError::FactoryFailed {
                name: self.inference_name.clone(),
                message: "no active model".to_string(),
            })?;

        let metadata = self.metadata(self.current_fleet_key(), &active);
        let session = InferenceSession::new(request_id, Arc::new(MetricsSink))
            .with_attributes(metadata.attributes());
        session.run(&active.inference, input, output);
        Ok(())
    }
}

impl<I> Inference for FleetManagedInference<I>
where
    I: Inference,
{
    type Input = <I as Inference>::Input;
    type Output = <I as Inference>::Output;

    fn infer(
        &self,
        session: &InferenceSession,
        input: InferenceInput<Self::Input>,
        output: InferenceOutput<Self::Output>,
    ) {
        let fleet_key = self.current_fleet_key();
        let request_span = tracing::info_span!(
            "fleet.inference",
            fleet_key = fleet_key.as_str(),
            inference_name = self.inference_name.as_str(),
        );
        let _request_guard = request_span.enter();

        if let Err(err) = self.maybe_sync_and_rollout() {
            output.error(Box::new(err)).ok();
            return;
        }

        let Some(active) = self.active() else {
            output
                .error(Box::new(FleetManagedInferenceError::FactoryFailed {
                    name: self.inference_name.clone(),
                    message: "no active model".to_string(),
                }))
                .ok();
            return;
        };

        // Scope the caller's session with fleet metadata so the inner model's telemetry is tagged.
        let session = session.with_attributes(self.metadata(fleet_key, &active).attributes());
        active.inference.infer(&session, input, output)
    }
}

fn normalized_model_version(version: &str) -> String {
    if version.is_empty() {
        "unknown".to_string()
    } else {
        version.to_string()
    }
}
