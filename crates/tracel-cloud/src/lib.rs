use std::error::Error;

use burn_central_client::BurnCentralCredentials;
use burn_central_client::Client;
use burn_central_client::ClientError;
use burn_central_client::Env;
use tracel_experiment::ExperimentJob;
use tracel_experiment::ExperimentRun;
use tracel_experiment::ExperimentRunHandleExt;

#[derive(Debug, Clone)]
pub struct CloudContext {
    pub client: Client,
    pub namespace: String,
    pub project: String,
}

// fn discover(env: Env) -> Result<Self, DiscoverError>  implementation for CloudContext could read credentials from env first, then fallback to the place that the CLI stores them.
// namespace and project could also be read from env, or fallback to some tracel TOML config file.
// The idea is that the user should be able to just do CloudContext::default() and have it work without having to set up anything else, as long as they have the CLI installed and configured.

impl CloudContext {
    pub fn new(
        credentials: BurnCentralCredentials,
        namespace: String,
        project: String,
        env: Env,
    ) -> Result<Self, ClientError> {
        let client = Client::new(env, &credentials)?;
        Ok(Self {
            client,
            namespace,
            project,
        })
    }

    pub fn experiment<T, F>(&self, f: F) -> ExperimentJob<T>
    where
        F: Fn(&ExperimentRun, T) -> Result<(), Box<dyn Error>> + Send + Sync + 'static,
    {
        let client = self.client.clone();
        let namespace = self.namespace.clone();
        let project = self.project.clone();

        let job_closure = move |input: T| {
            let experiment = Self::setup_experiment::<F>(&client, &namespace, &project)?;

            let handle = experiment.handle();
            let result = handle.in_scope(|| f(&experiment, input));

            match result {
                Ok(()) => experiment
                    .finish()
                    .map_err(|e| format!("Failed to finish experiment: {e}").into()),
                Err(e) => {
                    let msg = e.to_string();
                    let _ = experiment.fail(msg);
                    Err(e)
                }
            }
        };

        ExperimentJob::new(job_closure)
    }

    fn setup_experiment<F>(
        client: &Client,
        namespace: &str,
        project: &str,
    ) -> Result<ExperimentRun, String> {
        let digest = "46523358ec1646354ddab1cd8b93f2b920b44b24a26ea86c129d666d6bae2a5f".to_string();

        let _ = tracel_experiment::integration::tracing::try_init_tracing_subscriber();

        let experiment = ExperimentRun::cloud(
            client.clone(),
            namespace,
            project,
            digest,
            std::any::type_name::<F>().to_string(),
        )
        .map_err(|e| {
            use std::error::Error;
            let mut msg = format!("An error occured while creating the experiment: {e}");
            let mut src = e.source();
            while let Some(s) = src {
                msg.push_str(&format!("caused by: {s}"));
                src = s.source();
            }
            msg
        })?;

        Ok(experiment)
    }
}
