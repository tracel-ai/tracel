use std::io::Read;

use crate::{
    bundle::BundleSink, tools::path::normalize_bundle_path, upload::MultipartUploadSource,
};

/// A builder for creating bundles with multiple files
#[derive(Default, Clone)]
pub struct InMemoryBundleSources {
    files: Vec<PendingFile>,
}

impl InMemoryBundleSources {
    /// Create a new empty bundle sources
    pub fn new() -> Self {
        Self::default()
    }

    /// Add raw bytes as a file within the bundle at `dest_path`.
    pub fn add_bytes(mut self, bytes: Vec<u8>, dest_path: impl AsRef<str>) -> Self {
        self.files.push(PendingFile {
            dest_path: normalize_bundle_path(dest_path.as_ref()),
            source: bytes,
        });
        self
    }

    /// Add a file from a reader
    pub fn add_file<R: Read>(
        self,
        mut reader: R,
        dest_path: impl AsRef<str>,
    ) -> Result<Self, std::io::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        Ok(self.add_bytes(bytes, dest_path))
    }

    /// Get the files in this bundle sources
    pub fn files(&self) -> &Vec<PendingFile> {
        &self.files
    }

    /// Convert into the files vector
    pub fn into_files(self) -> Vec<PendingFile> {
        self.files
    }

    /// Check if the bundle is empty
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Get the number of files
    pub fn len(&self) -> usize {
        self.files.len()
    }
}

impl BundleSink for InMemoryBundleSources {
    fn put_file<R: Read>(&mut self, path: &str, reader: &mut R) -> Result<(), String> {
        let mut buf = Vec::new();
        reader
            .read_to_end(&mut buf)
            .map_err(|e| format!("Failed to read from source: {}", e))?;
        self.files.push(PendingFile {
            dest_path: normalize_bundle_path(path),
            source: buf,
        });
        Ok(())
    }
}

impl MultipartUploadSource for InMemoryBundleSources {
    fn file_len(&self, rel_path: &str) -> Result<u64, crate::upload::UploadError> {
        self.files
            .iter()
            .find(|f| f.dest_path() == rel_path)
            .map(|f| f.size() as u64)
            .ok_or_else(|| crate::upload::UploadError::MultipartReader {
                rel_path: rel_path.to_string(),
                source: format!("File not found in bundle sources: {}", rel_path).into(),
            })
    }

    fn open_part(
        &self,
        rel_path: &str,
        offset: u64,
        size: u64,
    ) -> Result<Box<dyn Read + Send>, crate::upload::UploadError> {
        let file = self
            .files
            .iter()
            .find(|f| f.dest_path() == rel_path)
            .ok_or_else(|| crate::upload::UploadError::MultipartReader {
                rel_path: rel_path.to_string(),
                source: format!("File not found in bundle sources: {}", rel_path).into(),
            })?;

        let data = file.source();
        let end = (offset + size) as usize;
        if end > data.len() {
            return Err(crate::upload::UploadError::MultipartReader {
                rel_path: rel_path.to_string(),
                source: format!(
                    "Requested part exceeds file size for {}: offset {} + size {} > file size {}",
                    rel_path,
                    offset,
                    size,
                    data.len()
                )
                .into(),
            });
        }

        Ok(Box::new(std::io::Cursor::new(
            data[offset as usize..end].to_vec(),
        )))
    }
}

/// A file that is pending to be added to a bundle
#[derive(Clone)]
pub struct PendingFile {
    pub dest_path: String, // path within the bundle (use forward slashes)
    pub source: Vec<u8>,
}

impl PendingFile {
    /// Get the destination path of this file
    pub fn dest_path(&self) -> &str {
        &self.dest_path
    }

    /// Get the source bytes of this file
    pub fn source(&self) -> &[u8] {
        &self.source
    }

    /// Get the size of this file
    pub fn size(&self) -> usize {
        self.source.len()
    }
}
