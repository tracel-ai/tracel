use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracel_console::{ConsoleError, FileSessionStore, ProjectHandle, SessionStoreError};
use tracel_datasets::DatasetRegistry;
use tracel_experiment::Experiments;
use tracel_experiment::local::LocalExperiments;
use tracel_inference::{InferenceModule, NoopInferenceProvider};
use tracel_models::ModelRegistry;
#[cfg(feature = "station")]
use tracel_station::Station;
use url::Url;

use crate::running_job;

const TRACEL_TARGET: &str = "TRACEL_TARGET";
const TRACEL_RUNS_DIR: &str = "TRACEL_RUNS_DIR";
#[cfg(feature = "station")]
const TRACEL_STATION_URL: &str = "TRACEL_STATION_URL";

const DEFAULT_RUNS_DIR: &str = "./runs";
#[cfg(feature = "station")]
const DEFAULT_STATION_URL: &str = "http://localhost:8000";

/// The values `TRACEL_TARGET` accepts, as an error lists them.
#[cfg(not(feature = "station"))]
const TARGETS: &str = "offline or console";
#[cfg(feature = "station")]
const TARGETS: &str = "offline, console or station";

/// Where a program records its experiments and reaches models, datasets and inference
/// telemetry.
///
/// Build one directly, or read it from the environment with [`Target::from_env`]. Neither
/// performs network I/O, and neither do the services a target builds: the first request that
/// needs a server reaches it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Target {
    /// Records experiments on this machine and reaches no server.
    Offline {
        /// The directory runs are recorded under.
        dir: PathBuf,
    },
    /// A project on the Tracel console. The services the target builds share its connection, so
    /// the credential is verified once.
    Console {
        /// The project experiments, models and datasets belong to.
        project: ProjectHandle,
    },
    /// A Tracel Station (requires the `station` feature). The services the target builds share
    /// its client.
    #[cfg(feature = "station")]
    Station {
        /// The Station experiments, models and datasets belong to.
        station: Station,
    },
}

impl Target {
    /// Reads the target the environment names, without performing network I/O.
    ///
    /// `TRACEL_TARGET` picks the kind of target, and the variables that kind needs fill it in. An unset or empty variable takes its default:
    ///
    /// | Variable | Value | Default |
    /// | --- | --- | --- |
    #[cfg_attr(
        not(feature = "station"),
        doc = "| `TRACEL_TARGET` | `offline` or `console` | `offline` |"
    )]
    #[cfg_attr(
        feature = "station",
        doc = "| `TRACEL_TARGET` | `offline`, `console` or `station` | `offline` |"
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
    /// The console target is the project [`ProjectHandle::from_env`] reads, and fails as it
    /// does, with [`TargetError::Console`]. Fails with [`TargetError::UnknownTarget`] when
    /// `TRACEL_TARGET` names no target, and with [`TargetError::InvalidSetting`] when
    /// `TRACEL_STATION_URL` is not an http or https URL.
    pub fn from_env() -> Result<Target, TargetError> {
        Self::from_vars(
            |name| std::env::var_os(name),
            Path::new("."),
            FileSessionStore::for_server,
        )
    }

    /// [`from_env`](Self::from_env), reading the variables from `lookup`, `tracel.toml` from
    /// `dir`, and the stored `tracel login` session from the store `sessions` gives for the
    /// console's URL.
    fn from_vars(
        lookup: impl Fn(&str) -> Option<OsString>,
        dir: &Path,
        sessions: impl FnOnce(&Url) -> Result<FileSessionStore, SessionStoreError>,
    ) -> Result<Target, TargetError> {
        let var = |name: &str| lookup(name).filter(|value| !value.is_empty());
        let target = var(TRACEL_TARGET).map(|value| value.to_string_lossy().into_owned());

        match target.as_deref() {
            None | Some("offline") => Ok(Target::Offline {
                dir: var(TRACEL_RUNS_DIR).map_or_else(|| DEFAULT_RUNS_DIR.into(), PathBuf::from),
            }),
            Some("console") => Ok(Target::Console {
                project: ProjectHandle::from_vars(&lookup, dir, sessions)?,
            }),
            #[cfg(feature = "station")]
            Some("station") => Ok(Target::Station {
                station: Station::connect(station_url(var(TRACEL_STATION_URL))?),
            }),
            #[cfg(not(feature = "station"))]
            Some("station") => Err(TargetError::FeatureRequired {
                target: "station",
                feature: "station",
            }),
            Some(value) => Err(TargetError::UnknownTarget {
                value: value.to_string(),
                expected: TARGETS,
            }),
        }
    }

    /// Returns the experiments this target records, without performing network I/O.
    ///
    /// Offline, the first run creates its directory.
    ///
    /// Built in a job, on the thread its adapter runs it on, the experiments are linked to that
    /// job, as its [`JobScope`](crate::runtime::JobScope) gives it, wherever runs are started from
    /// them: each experiment records the job's number, when its launcher gave one, as its
    /// `tracel.job_num` attribute and the job's input as its arguments, and prints where it
    /// records to stderr as it starts, such as `Recording experiment 42: <url>` or
    /// `Recording offline in <dir>`. Experiments built another way, such as
    /// [`ProjectHandle::experiments`] or [`Experiments::new`], are not linked unless the code links
    /// them through [`Experiments::attribute`], [`Experiments::arguments`] and
    /// [`Experiments::on_start`].
    pub fn experiments(&self) -> Result<Experiments, TargetError> {
        let experiments = match self {
            Target::Offline { dir } => Experiments::new(Arc::new(LocalExperiments::new(dir))),
            Target::Console { project } => project.experiments(),
            #[cfg(feature = "station")]
            Target::Station { station } => station.experiments(),
        };
        Ok(running_job::linked(experiments))
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
            Target::Console { project } => Ok(project.models()),
            #[cfg(feature = "station")]
            Target::Station { station } => Ok(station.models()),
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
            Target::Console { project } => Ok(project.datasets()),
            #[cfg(feature = "station")]
            Target::Station { station } => Ok(station.datasets()),
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
            Target::Console { project } => Ok(project.inference()),
            #[cfg(feature = "station")]
            Target::Station { .. } => Ok(noop_inference()),
        }
    }
}

/// The Station URL `value` names, `http://localhost:8000` when it is unset.
///
/// Fails with [`TargetError::InvalidSetting`] when it is not an http or https URL.
#[cfg(feature = "station")]
fn station_url(value: Option<OsString>) -> Result<Url, TargetError> {
    let url = value.map_or_else(
        || DEFAULT_STATION_URL.to_string(),
        |value| value.to_string_lossy().into_owned(),
    );
    match Url::parse(&url) {
        Ok(url) if matches!(url.scheme(), "http" | "https") => Ok(url),
        _ => Err(TargetError::InvalidSetting {
            variable: TRACEL_STATION_URL,
            value: url,
            expected: "an http or https URL, such as http://localhost:8000",
        }),
    }
}

fn noop_inference() -> InferenceModule {
    InferenceModule::new(Arc::new(NoopInferenceProvider::new()))
}

/// Errors reading a [`Target`] from the environment, or building a service from one.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TargetError {
    /// `TRACEL_TARGET` names no target.
    #[error("unknown TRACEL_TARGET value `{value}`: expected {expected}")]
    UnknownTarget {
        /// The value `TRACEL_TARGET` holds.
        value: String,
        /// The values it accepts.
        expected: &'static str,
    },
    /// `TRACEL_TARGET` names a target this build of `tracel` leaves out.
    #[error("TRACEL_TARGET={target} needs the `{feature}` feature of tracel")]
    FeatureRequired {
        /// The value `TRACEL_TARGET` holds.
        target: &'static str,
        /// The `tracel` feature that adds the target.
        feature: &'static str,
    },
    /// `TRACEL_STATION_URL` holds a value that is not an http or https URL.
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
    /// The console target cannot be filled in or reached, as the console says.
    #[error(transparent)]
    Console(#[from] ConsoleError),
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use tracel_console::ProjectRef;

    use super::*;

    fn target(vars: &[(&str, &str)]) -> Result<Target, TargetError> {
        let dir = tempfile::tempdir().unwrap();
        Target::from_vars(
            |name| {
                vars.iter()
                    .find(|(variable, _)| *variable == name)
                    .map(|(_, value)| OsString::from(value))
            },
            dir.path(),
            |_| Ok(FileSessionStore::new(dir.path().join("session.json"))),
        )
    }

    #[rstest]
    #[case::unset(&[])]
    #[case::empty(&[(TRACEL_TARGET, "")])]
    #[case::offline(&[(TRACEL_TARGET, "offline")])]
    #[case::runs_dir_empty(&[(TRACEL_RUNS_DIR, "")])]
    fn an_offline_target_records_under_the_default_runs_dir(#[case] vars: &[(&str, &str)]) {
        assert!(matches!(
            target(vars),
            Ok(Target::Offline { dir }) if dir == Path::new(DEFAULT_RUNS_DIR)
        ));
    }

    #[test]
    fn tracel_runs_dir_moves_offline_runs() {
        let runs = "/data/runs";

        assert!(matches!(
            target(&[(TRACEL_RUNS_DIR, runs)]),
            Ok(Target::Offline { dir }) if dir == Path::new(runs)
        ));
    }

    #[rstest]
    #[case::unknown("cloud")]
    #[case::capitalized("Offline")]
    #[case::upper_case("CONSOLE")]
    #[case::padded(" console")]
    fn an_unknown_target_is_an_error_naming_it(#[case] value: &str) {
        assert!(matches!(
            target(&[(TRACEL_TARGET, value)]),
            Err(TargetError::UnknownTarget { value: held, .. }) if held == value
        ));
    }

    #[cfg(not(feature = "station"))]
    #[test]
    fn a_station_target_needs_the_station_feature() {
        assert!(matches!(
            target(&[(TRACEL_TARGET, "station")]),
            Err(TargetError::FeatureRequired {
                feature: "station",
                ..
            })
        ));
    }

    #[cfg(feature = "station")]
    #[test]
    fn tracel_target_station_reaches_a_station() {
        assert!(matches!(
            target(&[(TRACEL_TARGET, "station")]),
            Ok(Target::Station { .. })
        ));
    }

    #[cfg(feature = "station")]
    #[rstest]
    #[case::unset(None, DEFAULT_STATION_URL)]
    #[case::set(Some("https://station.example.com"), "https://station.example.com")]
    fn tracel_station_url_names_the_station(#[case] value: Option<&str>, #[case] url: &str) {
        assert_eq!(
            station_url(value.map(OsString::from)).unwrap(),
            Url::parse(url).unwrap()
        );
    }

    #[cfg(feature = "station")]
    #[rstest]
    #[case::not_a_url("not a url")]
    #[case::not_http("ftp://localhost:8000")]
    fn a_station_url_that_is_not_http_is_an_invalid_setting_naming_it(#[case] value: &str) {
        assert!(matches!(
            target(&[(TRACEL_TARGET, "station"), (TRACEL_STATION_URL, value)]),
            Err(TargetError::InvalidSetting { variable: TRACEL_STATION_URL, value: held, .. })
                if held == value
        ));
    }

    #[test]
    fn building_offline_experiments_and_inference_creates_no_directory() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("runs");
        let offline = Target::Offline { dir: dir.clone() };

        let _ = (offline.experiments(), offline.inference());

        assert!(!dir.exists());
    }

    #[rstest]
    #[case::models("models", |offline: &Target| offline.models().err())]
    #[case::datasets("datasets", |offline: &Target| offline.datasets().err())]
    fn an_offline_target_serves_no_registry(
        #[case] service: &str,
        #[case] build: fn(&Target) -> Option<TargetError>,
    ) {
        let offline = Target::Offline {
            dir: PathBuf::from(DEFAULT_RUNS_DIR),
        };

        assert!(matches!(
            build(&offline),
            Some(TargetError::Unsupported { service: held, target: "offline" }) if held == service
        ));
    }

    #[test]
    fn a_console_target_is_the_project_its_variables_name() {
        let mnist = ProjectRef::new("alice", "mnist");

        let console = target(&[
            (TRACEL_TARGET, "console"),
            ("TRACEL_API_KEY", "tcl_key"),
            ("TRACEL_NAMESPACE", mnist.namespace.as_str()),
            ("TRACEL_PROJECT", mnist.name.as_str()),
        ]);

        assert!(matches!(
            console,
            Ok(Target::Console { project })
                if ProjectRef::new(project.owner(), project.name()) == mnist
        ));
    }

    #[test]
    fn a_console_target_without_a_credential_fails_as_the_console_does() {
        let console = target(&[
            (TRACEL_TARGET, "console"),
            ("TRACEL_NAMESPACE", "alice"),
            ("TRACEL_PROJECT", "mnist"),
        ]);

        assert!(matches!(
            console,
            Err(TargetError::Console(ConsoleError::NoCredentials))
        ));
    }
}
