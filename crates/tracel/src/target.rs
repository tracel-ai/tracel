use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use tracel_console::{Console, ConsoleError, CredentialSource, Env, ProjectHandle, ProjectRef};
use tracel_datasets::DatasetRegistry;
use tracel_experiment::Experiments;
use tracel_experiment::local::LocalExperiments;
use tracel_inference::{InferenceModule, NoopInferenceProvider};
use tracel_models::ModelRegistry;
#[cfg(feature = "station")]
use tracel_station::Station;
#[cfg(feature = "station")]
use url::Url;

const TRACEL_CONNECTION: &str = "TRACEL_CONNECTION";
const TRACEL_RUNS_DIR: &str = "TRACEL_RUNS_DIR";
const TRACEL_API_KEY: &str = "TRACEL_API_KEY";
const TRACEL_NAMESPACE: &str = "TRACEL_NAMESPACE";
const TRACEL_PROJECT: &str = "TRACEL_PROJECT";
#[cfg(feature = "station")]
const TRACEL_STATION_URL: &str = "TRACEL_STATION_URL";

const DEFAULT_RUNS_DIR: &str = "./runs";
#[cfg(feature = "station")]
const DEFAULT_STATION_URL: &str = "http://localhost:8000";

/// The values `TRACEL_CONNECTION` accepts, as an error lists them.
#[cfg(not(feature = "station"))]
const CONNECTIONS: &str = "offline or console";
#[cfg(feature = "station")]
const CONNECTIONS: &str = "offline, console or station";

/// Where a program records its experiments and reaches models, datasets and inference
/// telemetry.
///
/// Build one directly, or read it from the environment with [`Target::from_env`]. Neither
/// performs network I/O, and neither do the services a target builds: the first request that
/// needs a server reaches it. Each call to a service method binds a new connection; to share one
/// across services, build them from a [`ProjectHandle`] instead.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Target {
    /// Records experiments on this machine and reaches no server.
    Offline {
        /// The directory runs are recorded under.
        dir: PathBuf,
    },
    /// A project on the Tracel console.
    Console {
        /// The console to reach.
        env: Env,
        /// Where the credential comes from.
        credentials: CredentialSource,
        /// The project experiments, models and datasets belong to.
        project: ProjectRef,
    },
    /// A Tracel Station (requires the `station` feature).
    #[cfg(feature = "station")]
    Station {
        /// The Station's base URL.
        url: Url,
    },
}

impl Target {
    /// Reads the target the environment names, without performing network I/O.
    ///
    /// `TRACEL_CONNECTION` picks the kind of target, and the variables that kind needs fill it
    /// in. An unset or empty variable takes its default:
    ///
    /// | Variable | Value | Default |
    /// | --- | --- | --- |
    #[cfg_attr(
        not(feature = "station"),
        doc = "| `TRACEL_CONNECTION` | `offline` or `console` | `offline` |"
    )]
    #[cfg_attr(
        feature = "station",
        doc = "| `TRACEL_CONNECTION` | `offline`, `console` or `station` | `offline` |"
    )]
    /// | `TRACEL_RUNS_DIR` | the directory offline runs are recorded under | `./runs` |
    /// | `TRACEL_ENV` | the console, as [`env_from_environment`](crate::console::env_from_environment) reads it | `Production` |
    /// | `TRACEL_API_KEY` | an API key or a job token | the stored `tracel login` session |
    /// | `TRACEL_NAMESPACE`, `TRACEL_PROJECT` | the console project | `namespace` and `project` in `tracel.toml` |
    #[cfg_attr(
        feature = "station",
        doc = "| `TRACEL_STATION_URL` | the Station's base URL | `http://localhost:8000` |"
    )]
    ///
    /// Fails with [`TargetError::UnknownConnection`] when `TRACEL_CONNECTION` names no target, and
    /// with [`TargetError::MissingSetting`] or [`TargetError::InvalidSetting`] naming the variable
    /// to set when the target cannot be filled in.
    pub fn from_env() -> Result<Target, TargetError> {
        Self::from_vars(|name| std::env::var_os(name))
    }

    /// [`from_env`](Self::from_env) over the values `lookup` gives this crate's variables. The
    /// console's own variables are read by [`tracel_console`].
    fn from_vars(lookup: impl Fn(&str) -> Option<OsString>) -> Result<Target, TargetError> {
        let var = |name: &str| lookup(name).filter(|value| !value.is_empty());
        let connection = var(TRACEL_CONNECTION).map(|value| value.to_string_lossy().into_owned());

        match connection.as_deref() {
            None | Some("offline") => Ok(Target::Offline {
                dir: var(TRACEL_RUNS_DIR).map_or_else(|| DEFAULT_RUNS_DIR.into(), PathBuf::from),
            }),
            Some("console") => Ok(Target::Console {
                env: tracel_console::env_from_environment()?,
                credentials: CredentialSource::from_env(),
                project: ProjectRef::from_env()?,
            }),
            #[cfg(feature = "station")]
            Some("station") => {
                let url = var(TRACEL_STATION_URL).map_or_else(
                    || DEFAULT_STATION_URL.to_string(),
                    |value| value.to_string_lossy().into_owned(),
                );
                match Url::parse(&url) {
                    Ok(url) if matches!(url.scheme(), "http" | "https") => {
                        Ok(Target::Station { url })
                    }
                    _ => Err(TargetError::InvalidSetting {
                        variable: TRACEL_STATION_URL,
                        value: url,
                        expected: "an http or https URL, such as http://localhost:8000",
                    }),
                }
            }
            #[cfg(not(feature = "station"))]
            Some("station") => Err(TargetError::FeatureRequired {
                connection: "station",
                feature: "station",
            }),
            Some(value) => Err(TargetError::UnknownConnection {
                value: value.to_string(),
                expected: CONNECTIONS,
            }),
        }
    }

    /// Returns the experiments this target records, without performing network I/O.
    ///
    /// Offline, the first run creates its directory.
    pub fn experiments(&self) -> Result<Experiments, TargetError> {
        match self {
            Target::Offline { dir } => Ok(Experiments::new(Arc::new(LocalExperiments::new(dir)))),
            Target::Console {
                env,
                credentials,
                project,
            } => Ok(console_project(env, credentials, project)?.experiments()),
            #[cfg(feature = "station")]
            Target::Station { url } => Ok(Station::connect(url.clone()).experiments()),
        }
    }

    /// Returns the model registry this target reaches, without performing network I/O.
    ///
    /// Fails with [`TargetError::Unsupported`] for an offline target.
    pub fn models(&self) -> Result<ModelRegistry, TargetError> {
        match self {
            Target::Offline { .. } => Err(TargetError::Unsupported {
                service: "models",
                target: "offline",
            }),
            Target::Console {
                env,
                credentials,
                project,
            } => Ok(console_project(env, credentials, project)?.models()),
            #[cfg(feature = "station")]
            Target::Station { url } => Ok(Station::connect(url.clone()).models()),
        }
    }

    /// Returns the dataset registry this target reaches, without performing network I/O.
    ///
    /// Fails with [`TargetError::Unsupported`] for an offline target.
    pub fn datasets(&self) -> Result<DatasetRegistry, TargetError> {
        match self {
            Target::Offline { .. } => Err(TargetError::Unsupported {
                service: "datasets",
                target: "offline",
            }),
            Target::Console {
                env,
                credentials,
                project,
            } => Ok(console_project(env, credentials, project)?.datasets()),
            #[cfg(feature = "station")]
            Target::Station { url } => Ok(Station::connect(url.clone()).datasets()),
        }
    }

    /// Returns the inference module this target ships telemetry to, without performing network
    /// I/O.
    ///
    /// Only a console target ships inference telemetry; elsewhere, sessions discard what they
    /// record. Build the module once and reuse it, as [`ProjectHandle::inference`] explains.
    pub fn inference(&self) -> Result<InferenceModule, TargetError> {
        match self {
            Target::Offline { .. } => Ok(noop_inference()),
            Target::Console {
                env,
                credentials,
                project,
            } => Ok(console_project(env, credentials, project)?.inference()),
            #[cfg(feature = "station")]
            Target::Station { .. } => Ok(noop_inference()),
        }
    }
}

/// Binds `project` on the console `env` names, with the credential `credentials` provides.
fn console_project(
    env: &Env,
    credentials: &CredentialSource,
    project: &ProjectRef,
) -> Result<ProjectHandle, TargetError> {
    let console = Console::connect(env.clone(), credentials.resolve(env)?);
    Ok(console.project(&project.namespace, &project.name))
}

fn noop_inference() -> InferenceModule {
    InferenceModule::new(Arc::new(NoopInferenceProvider::new()))
}

/// Errors reading a [`Target`] from the environment, or building a service from one.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TargetError {
    /// `TRACEL_CONNECTION` names no target.
    #[error("unknown TRACEL_CONNECTION value `{value}`: expected {expected}")]
    UnknownConnection {
        /// The value `TRACEL_CONNECTION` holds.
        value: String,
        /// The values it accepts.
        expected: &'static str,
    },
    /// `TRACEL_CONNECTION` names a target this build of `tracel` leaves out.
    #[error("TRACEL_CONNECTION={connection} needs the `{feature}` feature of tracel")]
    FeatureRequired {
        /// The value `TRACEL_CONNECTION` holds.
        connection: &'static str,
        /// The `tracel` feature that adds the target.
        feature: &'static str,
    },
    /// A setting the target needs is not set.
    #[error("{variable} is not set{}", fallback(.variable))]
    MissingSetting {
        /// The environment variable to set.
        variable: &'static str,
    },
    /// An environment variable holds a value the SDK does not accept.
    #[error("invalid {variable} value `{value}`: expected {expected}")]
    InvalidSetting {
        /// The environment variable.
        variable: &'static str,
        /// The value it holds.
        value: String,
        /// The values it accepts.
        expected: &'static str,
    },
    /// The target does not serve this service.
    #[error("the {target} target does not serve {service}")]
    Unsupported {
        /// The service asked for.
        service: &'static str,
        /// The kind of target that does not serve it.
        target: &'static str,
    },
    /// The console settings could not be read for another reason, such as a stored sign-in
    /// that cannot be read.
    #[error(transparent)]
    Console(ConsoleError),
}

/// What else provides a setting whose variable is unset, as [`TargetError::MissingSetting`] says.
fn fallback(variable: &str) -> &'static str {
    match variable {
        TRACEL_API_KEY => ", and there is no `tracel login` sign-in for this console",
        TRACEL_NAMESPACE => ", and tracel.toml names no namespace",
        TRACEL_PROJECT => ", and tracel.toml names no project",
        _ => "",
    }
}

impl From<ConsoleError> for TargetError {
    fn from(error: ConsoleError) -> Self {
        match error {
            ConsoleError::NoCredentials => Self::MissingSetting {
                variable: TRACEL_API_KEY,
            },
            ConsoleError::NoNamespace => Self::MissingSetting {
                variable: TRACEL_NAMESPACE,
            },
            ConsoleError::NoProject => Self::MissingSetting {
                variable: TRACEL_PROJECT,
            },
            ConsoleError::InvalidSetting {
                variable,
                value,
                expected,
            } => Self::InvalidSetting {
                variable,
                value,
                expected,
            },
            error => Self::Console(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tracel_console::TracelCredentials;

    use super::*;

    fn target(vars: &[(&str, &str)]) -> Result<Target, TargetError> {
        Target::from_vars(|name| {
            vars.iter()
                .find(|(variable, _)| *variable == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    fn offline_dir(target: Result<Target, TargetError>) -> PathBuf {
        match target {
            Ok(Target::Offline { dir }) => dir,
            other => panic!("expected an offline target, got {other:?}"),
        }
    }

    #[test]
    fn an_unset_or_empty_connection_records_offline_under_runs() {
        assert_eq!(offline_dir(target(&[])), Path::new("./runs"));
        assert_eq!(
            offline_dir(target(&[(TRACEL_CONNECTION, "")])),
            Path::new("./runs")
        );
        assert_eq!(
            offline_dir(target(&[(TRACEL_CONNECTION, "offline")])),
            Path::new("./runs")
        );
    }

    #[test]
    fn tracel_runs_dir_moves_offline_runs() {
        assert_eq!(
            offline_dir(target(&[(TRACEL_RUNS_DIR, "/data/runs")])),
            Path::new("/data/runs")
        );
        assert_eq!(
            offline_dir(target(&[(TRACEL_RUNS_DIR, "")])),
            Path::new("./runs")
        );
    }

    #[test]
    fn an_unknown_connection_names_the_value_and_the_accepted_ones() {
        for value in ["cloud", "Offline", "CONSOLE", " console"] {
            let error = target(&[(TRACEL_CONNECTION, value)]).unwrap_err();

            assert!(
                matches!(
                    &error,
                    TargetError::UnknownConnection { value: held, .. } if held == value
                ),
                "{value}: {error:?}"
            );
            let message = error.to_string();
            assert!(message.contains("TRACEL_CONNECTION"), "{message}");
            assert!(message.contains(value), "{message}");
            assert!(message.contains("offline"), "{message}");
            assert!(message.contains("console"), "{message}");
        }
    }

    #[cfg(not(feature = "station"))]
    #[test]
    fn a_station_connection_says_it_needs_the_station_feature() {
        let error = target(&[(TRACEL_CONNECTION, "station")]).unwrap_err();

        assert!(matches!(
            error,
            TargetError::FeatureRequired {
                feature: "station",
                ..
            }
        ));
        assert!(error.to_string().contains("`station` feature"), "{error}");
    }

    #[cfg(feature = "station")]
    #[test]
    fn a_station_connection_reads_tracel_station_url() {
        let station_url = |vars: &[(&str, &str)]| match target(vars) {
            Ok(Target::Station { url }) => url.to_string(),
            other => panic!("expected a station target, got {other:?}"),
        };

        assert_eq!(
            station_url(&[(TRACEL_CONNECTION, "station")]),
            "http://localhost:8000/"
        );
        assert_eq!(
            station_url(&[
                (TRACEL_CONNECTION, "station"),
                (TRACEL_STATION_URL, "https://station.example.com")
            ]),
            "https://station.example.com/"
        );
    }

    #[cfg(feature = "station")]
    #[test]
    fn an_invalid_station_url_names_the_variable() {
        for value in ["not a url", "localhost:8000", "ftp://localhost:8000"] {
            let error =
                target(&[(TRACEL_CONNECTION, "station"), (TRACEL_STATION_URL, value)]).unwrap_err();

            assert!(
                matches!(
                    &error,
                    TargetError::InvalidSetting { variable: "TRACEL_STATION_URL", value: held, .. }
                        if held == value
                ),
                "{value}: {error:?}"
            );
            assert!(error.to_string().contains("TRACEL_STATION_URL"), "{error}");
        }
    }

    #[test]
    fn an_offline_target_serves_experiments_and_inference_without_touching_the_disk() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("runs");
        let offline = Target::Offline { dir: dir.clone() };

        assert!(offline.experiments().is_ok());
        assert!(offline.inference().is_ok());
        assert!(!dir.exists());
    }

    #[test]
    fn an_offline_target_does_not_serve_models_or_datasets() {
        let offline = Target::Offline {
            dir: PathBuf::from("./runs"),
        };

        assert!(matches!(
            offline.models(),
            Err(TargetError::Unsupported {
                service: "models",
                target: "offline"
            })
        ));
        assert!(matches!(
            offline.datasets(),
            Err(TargetError::Unsupported {
                service: "datasets",
                target: "offline"
            })
        ));
    }

    #[test]
    fn a_console_target_builds_its_services_without_a_request() {
        let console = Target::Console {
            env: Env::Development,
            credentials: CredentialSource::Explicit(TracelCredentials::api_key(
                "tcl_key_not_checked_until_the_first_request",
            )),
            project: ProjectRef::new("owner", "project"),
        };

        assert!(console.experiments().is_ok());
        assert!(console.models().is_ok());
        assert!(console.datasets().is_ok());
        assert!(console.inference().is_ok());
    }

    #[test]
    fn a_missing_console_setting_names_the_variable_to_set() {
        for (error, variable, fallback) in [
            (ConsoleError::NoCredentials, TRACEL_API_KEY, "tracel login"),
            (ConsoleError::NoNamespace, TRACEL_NAMESPACE, "tracel.toml"),
            (ConsoleError::NoProject, TRACEL_PROJECT, "tracel.toml"),
        ] {
            let error = TargetError::from(error);

            assert!(
                matches!(error, TargetError::MissingSetting { variable: held } if held == variable),
                "{error:?}"
            );
            assert!(error.to_string().contains(variable), "{error}");
            assert!(error.to_string().contains(fallback), "{error}");
        }
    }
}
