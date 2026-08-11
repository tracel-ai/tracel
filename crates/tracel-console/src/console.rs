use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracel_artifact::bundle::BundleSink;
use tracel_artifact::download::{
    ArtifactDownloadFile, DownloadObserver, download_artifacts_to_sink_with_client_and_observer,
};
use tracel_artifact::{FileTransferClient, ReqwestTransferClient};
use url::Url;

use crate::{
    ConsoleError, Model, ModelVersion, Namespace, NamespaceKind, Organization, Page, Project, User,
    VersionId, normalize_base_url,
};

/// A session identifier issued by a Tracel console.
///
/// Debug formatting is redacted so ordinary diagnostic output does not disclose credentials.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionToken(String);

impl SessionToken {
    /// Wraps a token obtained from authentication or persistent storage.
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// Exposes the token when a caller explicitly needs to persist it.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    /// Consumes the wrapper and returns the token.
    pub fn into_secret(self) -> String {
        self.0
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionToken([REDACTED])")
    }
}

/// Authentication state used by a [`Console`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    /// Make only requests available to anonymous callers.
    Anonymous,
    /// Send the supplied session token as the console's `id` cookie.
    Session(SessionToken),
}

/// A blocking client rooted at one Tracel console URL.
#[derive(Clone)]
pub struct Console {
    inner: Arc<ConsoleInner>,
}

struct ConsoleInner {
    client: tracel_client::Client,
}

impl Console {
    /// Creates a console client without making a network request.
    ///
    /// The exact path prefix is preserved and normalized with a trailing slash so versioned API
    /// paths are joined underneath it.
    pub fn new(url: impl AsRef<str>, auth: Auth) -> Result<Self, ConsoleError> {
        let url = normalize_base_url(url.as_ref())?;
        let client = match auth {
            Auth::Anonymous => tracel_client::Client::anonymous(url),
            Auth::Session(token) => {
                tracel_client::Client::from_url_with_session_token(url, token.expose_secret())
            }
        };

        Ok(Self {
            inner: Arc::new(ConsoleInner { client }),
        })
    }

    /// Returns the normalized console API base URL.
    pub fn base_url(&self) -> &Url {
        self.inner.client.base_url()
    }

    /// Returns the current user, or `None` when the session is absent or dead.
    ///
    /// A dead session is represented by the console as a successful `null` response and remains a
    /// value rather than [`ConsoleError::SessionExpired`].
    pub fn me(&self) -> Result<Option<User>, ConsoleError> {
        self.inner
            .client
            .get_current_user()
            .map(|user| {
                user.map(|user| User {
                    id: user._id,
                    username: user.username,
                    email: user.email,
                    namespace: Namespace::user(user.namespace),
                })
            })
            .map_err(Into::into)
    }

    /// Destroys the current session on the console.
    pub fn logout(&self) -> Result<(), ConsoleError> {
        self.inner.client.logout().map_err(Into::into)
    }

    /// Lists organizations available to the current session.
    pub fn organizations(&self) -> Result<Vec<Organization>, ConsoleError> {
        self.inner
            .client
            .get_user_organizations()
            .map(|response| {
                response
                    .organizations
                    .into_iter()
                    .map(|organization| Organization {
                        name: organization.name,
                        namespace: Namespace::organization(organization.namespace),
                    })
                    .collect()
            })
            .map_err(Into::into)
    }

    /// Lists visible projects owned by a user or organization namespace.
    pub fn projects_of(
        &self,
        namespace: impl AsRef<Namespace>,
    ) -> Result<Vec<Project>, ConsoleError> {
        let namespace = namespace.as_ref();
        let projects = match namespace.kind {
            NamespaceKind::User => self.inner.client.list_user_projects(&namespace.name),
            NamespaceKind::Organization => self
                .inner
                .client
                .list_organization_projects(&namespace.name),
        }?;

        projects.into_iter().map(Project::try_from).collect()
    }

    /// Creates a project handle without performing I/O.
    pub fn project<O, P>(&self, (owner, project): (O, P)) -> ProjectHandle
    where
        O: Into<String>,
        P: Into<String>,
    {
        ProjectHandle {
            inner: Arc::clone(&self.inner),
            owner: owner.into(),
            project: project.into(),
        }
    }
}

impl fmt::Debug for Console {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Console")
            .field("base_url", &self.base_url())
            .finish_non_exhaustive()
    }
}

/// A cheap view of one project that shares its console client's session.
#[derive(Clone)]
pub struct ProjectHandle {
    inner: Arc<ConsoleInner>,
    owner: String,
    project: String,
}

impl ProjectHandle {
    /// Returns the project's owner namespace.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Returns the project name.
    pub fn name(&self) -> &str {
        &self.project
    }

    /// Fetches project details.
    ///
    /// Private and nonexistent projects both return [`ConsoleError::NotVisible`] because the
    /// console intentionally does not reveal which case applies.
    pub fn get(&self) -> Result<Project, ConsoleError> {
        self.inner
            .client
            .get_project(&self.owner, &self.project)
            .map_err(ConsoleError::from)
            .and_then(Project::try_from)
    }

    /// Lists models registered in the project.
    pub fn models(&self) -> Result<Page<Model>, ConsoleError> {
        self.inner
            .client
            .list_models(&self.owner, &self.project)
            .map(|response| Page {
                items: response.items.into_iter().map(Model::from).collect(),
                total: response.total,
            })
            .map_err(Into::into)
    }

    /// Creates a model handle without performing I/O.
    pub fn model(&self, name: impl Into<String>) -> ModelHandle {
        ModelHandle {
            inner: Arc::clone(&self.inner),
            owner: self.owner.clone(),
            project: self.project.clone(),
            model: name.into(),
        }
    }
}

impl fmt::Debug for ProjectHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectHandle")
            .field("owner", &self.owner)
            .field("project", &self.project)
            .finish()
    }
}

/// A cheap view of one registered model that shares its console client's session.
#[derive(Clone)]
pub struct ModelHandle {
    inner: Arc<ConsoleInner>,
    owner: String,
    project: String,
    model: String,
}

impl ModelHandle {
    /// Returns the model name.
    pub fn name(&self) -> &str {
        &self.model
    }

    /// Fetches model details.
    pub fn get(&self) -> Result<Model, ConsoleError> {
        self.inner
            .client
            .get_model(&self.owner, &self.project, &self.model)
            .map(Model::from)
            .map_err(Into::into)
    }

    /// Lists published model versions.
    pub fn versions(&self) -> Result<Page<ModelVersion>, ConsoleError> {
        let response =
            self.inner
                .client
                .list_model_versions(&self.owner, &self.project, &self.model)?;
        let items = response
            .items
            .into_iter()
            .map(ModelVersion::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Page {
            items,
            total: response.total,
        })
    }

    /// Creates a version handle from an opaque version identity without performing I/O.
    pub fn version(&self, id: VersionId) -> VersionHandle {
        VersionHandle {
            inner: Arc::clone(&self.inner),
            owner: self.owner.clone(),
            project: self.project.clone(),
            model: self.model.clone(),
            id,
        }
    }
}

impl fmt::Debug for ModelHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModelHandle")
            .field("owner", &self.owner)
            .field("project", &self.project)
            .field("model", &self.model)
            .finish()
    }
}

/// A cheap view of one published model version that shares its console client's session.
#[derive(Clone)]
pub struct VersionHandle {
    inner: Arc<ConsoleInner>,
    owner: String,
    project: String,
    model: String,
    id: VersionId,
}

impl VersionHandle {
    /// Returns the opaque version identity.
    pub fn id(&self) -> &VersionId {
        &self.id
    }

    /// Fetches version metadata and its typed manifest file listing.
    pub fn get(&self) -> Result<ModelVersion, ConsoleError> {
        let version = self.resolve_route_version()?;
        self.inner
            .client
            .get_model_version(&self.owner, &self.project, &self.model, version)
            .map_err(ConsoleError::from)
            .and_then(ModelVersion::try_from)
    }

    /// Presigns and downloads every version file into a caller-owned sink.
    ///
    /// Transfers report progress synchronously through `observer` and verify every announced size
    /// and checksum before reporting completion. A sink may stop reading and return an error at any
    /// point, which lets it share a cancellation flag with the observer and abort mid-file.
    pub fn download<S, O>(&self, dest: &mut S, observer: &mut O) -> Result<(), ConsoleError>
    where
        S: BundleSink,
        O: DownloadObserver,
    {
        let version = self.resolve_route_version()?;
        let response = self.inner.client.presign_model_download(
            &self.owner,
            &self.project,
            &self.model,
            version,
        )?;
        let files = response
            .files
            .into_iter()
            .map(|file| ArtifactDownloadFile {
                rel_path: file.rel_path,
                url: file.url,
                size_bytes: Some(file.size_bytes),
                checksum: Some(file.checksum),
            })
            .collect::<Vec<_>>();
        transfer_files(&ReqwestTransferClient::new(), dest, observer, &files)
    }

    fn resolve_route_version(&self) -> Result<u32, ConsoleError> {
        if let Some(version) = self.id.route_version() {
            return Ok(version);
        }

        let response =
            self.inner
                .client
                .list_model_versions(&self.owner, &self.project, &self.model)?;
        response
            .items
            .into_iter()
            .find_map(|version| (version.id == self.id.as_str()).then_some(version.version))
            .ok_or(ConsoleError::NotVisible)
    }
}

impl fmt::Debug for VersionHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VersionHandle")
            .field("owner", &self.owner)
            .field("project", &self.project)
            .field("model", &self.model)
            .field("id", &self.id)
            .finish()
    }
}

fn transfer_files<FTC, S, O>(
    client: &FTC,
    dest: &mut S,
    observer: &mut O,
    files: &[ArtifactDownloadFile],
) -> Result<(), ConsoleError>
where
    FTC: FileTransferClient,
    S: BundleSink,
    O: DownloadObserver,
{
    download_artifacts_to_sink_with_client_and_observer(client, dest, files, observer)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{Cursor, Read};
    use std::sync::Arc;

    use sha2::Digest;
    use tracel_artifact::TransferError;
    use tracel_artifact::bundle::InMemoryBundleSources;
    use tracel_artifact::download::DownloadError;

    use super::*;

    #[derive(Clone)]
    struct FakeTransferClient {
        files: Arc<HashMap<String, Vec<u8>>>,
    }

    impl FakeTransferClient {
        fn new(files: HashMap<String, Vec<u8>>) -> Self {
            Self {
                files: Arc::new(files),
            }
        }
    }

    impl FileTransferClient for FakeTransferClient {
        fn put_reader<R: Read + Send + 'static>(
            &self,
            _url: &str,
            _reader: R,
            _size_bytes: u64,
        ) -> Result<(), TransferError> {
            Ok(())
        }

        fn get_reader(&self, url: &str) -> Result<Box<dyn Read + Send>, TransferError> {
            self.files
                .get(url)
                .cloned()
                .map(Cursor::new)
                .map(|reader| Box::new(reader) as Box<dyn Read + Send>)
                .ok_or_else(|| TransferError::Transport(format!("missing fake URL: {url}")))
        }
    }

    #[derive(Default)]
    struct RecordingObserver {
        progress: Vec<u64>,
        completed: Vec<u64>,
    }

    impl DownloadObserver for RecordingObserver {
        fn file_progress(&mut self, _rel_path: &str, downloaded_bytes: u64) {
            self.progress.push(downloaded_bytes);
        }

        fn file_completed(&mut self, _rel_path: &str, downloaded_bytes: u64) {
            self.completed.push(downloaded_bytes);
        }
    }

    struct CancellingSink;

    impl BundleSink for CancellingSink {
        fn put_file<R: Read>(&mut self, _path: &str, reader: &mut R) -> Result<(), String> {
            let mut buffer = [0; 3];
            reader
                .read_exact(&mut buffer)
                .map_err(|error| error.to_string())?;
            Err("cancelled".to_string())
        }
    }

    fn checksum(bytes: &[u8]) -> String {
        let mut hasher = sha2::Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    #[test]
    fn console_normalizes_root_and_path_prefix_urls() {
        let root = Console::new("http://localhost:9001", Auth::Anonymous).unwrap();
        let api = Console::new("https://console.example/api", Auth::Anonymous).unwrap();
        let already_normalized =
            Console::new("https://console.example/api/", Auth::Anonymous).unwrap();

        assert_eq!(root.base_url().as_str(), "http://localhost:9001/");
        assert_eq!(api.base_url().as_str(), "https://console.example/api/");
        assert_eq!(
            already_normalized.base_url().as_str(),
            "https://console.example/api/"
        );
        assert_eq!(
            api.base_url().join("v1/user").unwrap().as_str(),
            "https://console.example/api/v1/user"
        );
    }

    #[test]
    fn console_rejects_non_http_and_qualified_base_urls() {
        assert!(Console::new("file:///tmp/console", Auth::Anonymous).is_err());
        assert!(Console::new("https://console.example/api?q=1", Auth::Anonymous).is_err());
    }

    #[test]
    fn download_reports_progress_and_verifies_size_and_checksum() {
        let bytes = b"verified payload".to_vec();
        let files = [ArtifactDownloadFile {
            rel_path: "weights.bpk".to_string(),
            url: "fake://weights".to_string(),
            size_bytes: Some(bytes.len() as u64),
            checksum: Some(checksum(&bytes)),
        }];
        let client = FakeTransferClient::new(HashMap::from([(
            "fake://weights".to_string(),
            bytes.clone(),
        )]));
        let mut dest = InMemoryBundleSources::new();
        let mut observer = RecordingObserver::default();

        transfer_files(&client, &mut dest, &mut observer, &files).unwrap();

        assert_eq!(observer.progress.last(), Some(&(bytes.len() as u64)));
        assert_eq!(observer.completed, vec![bytes.len() as u64]);
    }

    #[test]
    fn download_surfaces_verification_failures() {
        let bytes = b"payload".to_vec();
        let files = [ArtifactDownloadFile {
            rel_path: "weights.bpk".to_string(),
            url: "fake://weights".to_string(),
            size_bytes: Some(bytes.len() as u64 + 1),
            checksum: Some(checksum(&bytes)),
        }];
        let client =
            FakeTransferClient::new(HashMap::from([("fake://weights".to_string(), bytes)]));
        let mut dest = InMemoryBundleSources::new();
        let mut observer = RecordingObserver::default();

        let error = transfer_files(&client, &mut dest, &mut observer, &files).unwrap_err();

        assert!(matches!(
            error,
            ConsoleError::Download(DownloadError::SizeMismatch { .. })
        ));
        assert!(observer.completed.is_empty());
    }

    #[test]
    fn a_sink_can_abort_during_a_file() {
        let bytes = b"a payload longer than one sink read".to_vec();
        let files = [ArtifactDownloadFile {
            rel_path: "weights.bpk".to_string(),
            url: "fake://weights".to_string(),
            size_bytes: Some(bytes.len() as u64),
            checksum: Some(checksum(&bytes)),
        }];
        let client = FakeTransferClient::new(HashMap::from([(
            "fake://weights".to_string(),
            bytes.clone(),
        )]));
        let mut dest = CancellingSink;
        let mut observer = RecordingObserver::default();

        let error = transfer_files(&client, &mut dest, &mut observer, &files).unwrap_err();

        assert!(matches!(
            error,
            ConsoleError::Download(DownloadError::TargetError(_))
        ));
        assert_eq!(observer.progress, vec![3]);
        assert!(observer.completed.is_empty());
    }

    #[test]
    fn session_tokens_are_redacted_in_debug_output() {
        let token = SessionToken::new("do-not-log-me");

        assert_eq!(format!("{token:?}"), "SessionToken([REDACTED])");
        assert!(!format!("{:?}", Auth::Session(token)).contains("do-not-log-me"));
    }
}
