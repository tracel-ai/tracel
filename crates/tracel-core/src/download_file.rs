use tracel_artifact::download::ArtifactDownloadFile;

pub(crate) fn artifact_download_file(rel_path: String, url: String) -> ArtifactDownloadFile {
    ArtifactDownloadFile {
        rel_path,
        url,
        size_bytes: None,
        checksum: None,
    }
}

pub(crate) fn artifact_download_file_with_verification(
    rel_path: String,
    url: String,
    size_bytes: u64,
    checksum: String,
) -> ArtifactDownloadFile {
    ArtifactDownloadFile {
        rel_path,
        url,
        size_bytes: Some(size_bytes),
        checksum: Some(checksum),
    }
}
