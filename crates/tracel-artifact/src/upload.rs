//! This module provides utilities for uploading artifact files from any source to any target bundle sink using multipart uploads with presigned URLs.
//!
//! The upload process can be customized with any implementation of the TransferClient trait (e.g. for custom HTTP clients, authentication, retries, etc), and multipart file sources can be abstracted behind the MultipartUploadSource trait for maximum flexibility (e.g. to support streaming from large files without loading them fully into memory).

use crate::TransferClient;
use crate::transfer::{TransferError, TransferObserver, reader_stream};
use std::collections::HashSet;
use std::io::Read;
use tracel_task::DynFuture;

/// Errors that can occur during artifact file uploads.
#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    /// The caller cancelled while a file was being uploaded.
    #[error("upload cancelled while transferring {rel_path}")]
    Cancelled { rel_path: String },
    /// Errors from the transfer client (e.g. network errors, HTTP errors).
    #[error("transfer error for part {part_index} of {total_parts} for {rel_path}: {source}")]
    Transfer {
        part_index: usize,
        total_parts: usize,
        rel_path: String,
        #[source]
        source: TransferError,
    },
    /// Errors related to invalid multipart upload plans (e.g. duplicate paths, invalid part numbering).
    #[error("invalid multipart upload plan: {0}")]
    InvalidMultipart(String),
    /// Errors related to multipart reader issues (e.g. file access errors).
    #[error("multipart reader error for {rel_path}: {source}")]
    MultipartReader {
        rel_path: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// One multipart upload part descriptor.
#[derive(Debug, Clone)]
pub struct MultipartUploadPart {
    pub part: u32,
    pub url: String,
    pub size_bytes: u64,
}

/// One file multipart upload descriptor.
#[derive(Debug, Clone)]
pub struct MultipartUploadFile {
    pub rel_path: String,
    pub parts: Vec<MultipartUploadPart>,
}

/// Source abstraction for multipart uploads.
pub trait MultipartUploadSource: Send + Sync {
    /// Return the file length in bytes for a relative path.
    fn file_len(&self, rel_path: &str) -> Result<u64, UploadError>;

    /// Open a reader for one file chunk.
    fn open_part(
        &self,
        rel_path: &str,
        offset: u64,
        size: u64,
    ) -> Result<Box<dyn Read + Send>, UploadError>;
}

/// So a borrowed source can be handed to code that wants to own one.
impl<S: MultipartUploadSource + ?Sized> MultipartUploadSource for &S {
    fn file_len(&self, rel_path: &str) -> Result<u64, UploadError> {
        (**self).file_len(rel_path)
    }

    fn open_part(
        &self,
        rel_path: &str,
        offset: u64,
        size: u64,
    ) -> Result<Box<dyn Read + Send>, UploadError> {
        (**self).open_part(rel_path, offset, size)
    }
}

/// Uploads `files` from `source` part by part to their presigned URLs, reporting progress and
/// honouring cancellation through `observer`.
pub async fn upload_multipart<C, S, O>(
    client: &C,
    source: &S,
    files: &[MultipartUploadFile],
    observer: &mut O,
) -> Result<(), UploadError>
where
    C: TransferClient,
    S: MultipartUploadSource + ?Sized,
    O: TransferObserver + ?Sized,
{
    let mut seen = HashSet::new();

    for file in files {
        if !seen.insert(file.rel_path.clone()) {
            return Err(UploadError::InvalidMultipart(format!(
                "Duplicate multipart upload descriptor for {}",
                file.rel_path
            )));
        }

        if observer.is_cancelled() {
            return Err(UploadError::Cancelled {
                rel_path: file.rel_path.clone(),
            });
        }

        // Boxed so a caller's `dyn` arguments do not run into rust-lang/rust#100013.
        let upload: DynFuture<'_, Result<(), UploadError>> = Box::pin(upload_file_parts(
            client,
            source,
            &file.rel_path,
            &file.parts,
            observer,
        ));
        upload.await?;
    }

    Ok(())
}

async fn upload_file_parts<C, S, O>(
    client: &C,
    source: &S,
    rel_path: &str,
    parts: &[MultipartUploadPart],
    observer: &mut O,
) -> Result<(), UploadError>
where
    C: TransferClient,
    S: MultipartUploadSource + ?Sized,
    O: TransferObserver + ?Sized,
{
    let file_len = source.file_len(rel_path)?;
    observer.file_started(rel_path, Some(file_len));

    let mut part_indices: Vec<usize> = (0..parts.len()).collect();
    part_indices.sort_by_key(|&i| parts[i].part);

    for (i, &part_idx) in part_indices.iter().enumerate() {
        let part = &parts[part_idx];
        if part.part != (i as u32 + 1) {
            return Err(UploadError::InvalidMultipart(format!(
                "Invalid part numbering for {}: expected {}, got {}",
                rel_path,
                i + 1,
                part.part
            )));
        }
    }

    let mut offset = 0u64;

    for (part_index, &part_idx) in part_indices.iter().enumerate() {
        let part = &parts[part_idx];
        let size = part.size_bytes;

        if offset + size > file_len {
            return Err(UploadError::InvalidMultipart(format!(
                "Part {} exceeds file length for {}",
                part_index + 1,
                rel_path
            )));
        }

        if observer.is_cancelled() {
            return Err(UploadError::Cancelled {
                rel_path: rel_path.to_string(),
            });
        }

        let reader = source.open_part(rel_path, offset, size)?;
        client
            .put(&part.url, reader_stream(reader), size)
            .await
            .map_err(|e| UploadError::Transfer {
                part_index: part_index + 1,
                total_parts: parts.len(),
                rel_path: rel_path.to_string(),
                source: e,
            })?;

        offset += size;
        observer.file_progress(rel_path, offset);
    }

    if offset != file_len {
        return Err(UploadError::InvalidMultipart(format!(
            "Multipart size mismatch for {} (uploaded {}, expected {})",
            rel_path, offset, file_len
        )));
    }

    observer.file_completed(rel_path, offset);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures::{Stream, TryStreamExt};
    use std::collections::HashMap;
    use std::io::{self, Cursor, Read};
    use std::sync::{Arc, Mutex};
    use tracel_task::MaybeSend;

    #[derive(Clone, Default)]
    struct MockClient {
        #[allow(clippy::type_complexity)]
        puts: Arc<Mutex<Vec<(String, u64, Vec<u8>)>>>,
    }

    impl TransferClient for MockClient {
        type Body = futures::stream::Empty<Result<Bytes, TransferError>>;

        async fn get(
            &self,
            _url: &str,
            _expected_size_bytes: Option<u64>,
        ) -> Result<Self::Body, TransferError> {
            Err(TransferError::Transport(
                "get should not be used in upload tests".to_string(),
            ))
        }

        async fn put<B>(&self, url: &str, body: B, size_bytes: u64) -> Result<(), TransferError>
        where
            B: Stream<Item = Result<Bytes, io::Error>> + MaybeSend + 'static,
        {
            let bytes = body
                .map_ok(|chunk| chunk.to_vec())
                .try_concat()
                .await
                .map_err(|e| TransferError::Transport(e.to_string()))?;
            self.puts
                .lock()
                .expect("lock puts")
                .push((url.to_string(), size_bytes, bytes));
            Ok(())
        }
    }

    fn upload(
        client: &MockClient,
        source: &MockSource,
        files: &[MultipartUploadFile],
    ) -> Result<(), UploadError> {
        futures::executor::block_on(upload_multipart(client, source, files, &mut ()))
    }

    struct MockSource {
        files: HashMap<String, Vec<u8>>,
    }

    impl MockSource {
        fn new(files: HashMap<String, Vec<u8>>) -> Self {
            Self { files }
        }
    }

    impl MultipartUploadSource for MockSource {
        fn file_len(&self, rel_path: &str) -> Result<u64, UploadError> {
            let bytes = self.files.get(rel_path).ok_or_else(|| {
                UploadError::InvalidMultipart(format!("missing file: {rel_path}"))
            })?;
            Ok(bytes.len() as u64)
        }

        fn open_part(
            &self,
            rel_path: &str,
            offset: u64,
            size: u64,
        ) -> Result<Box<dyn Read + Send>, UploadError> {
            let bytes = self.files.get(rel_path).ok_or_else(|| {
                UploadError::InvalidMultipart(format!("missing file: {rel_path}"))
            })?;
            let start = offset as usize;
            let end = (offset + size) as usize;
            let slice = bytes
                .get(start..end)
                .ok_or_else(|| UploadError::MultipartReader {
                    rel_path: rel_path.to_string(),
                    source: format!(
                        "invalid part range [{start}..{end}) for file of len {}",
                        bytes.len()
                    )
                    .into(),
                })?;
            Ok(Box::new(Cursor::new(slice.to_vec())))
        }
    }

    #[test]
    fn rejects_non_contiguous_part_numbering() {
        let client = MockClient::default();
        let source = MockSource::new(HashMap::from([(
            "weights.bin".to_string(),
            b"abcd".to_vec(),
        )]));
        let files = vec![MultipartUploadFile {
            rel_path: "weights.bin".to_string(),
            parts: vec![
                MultipartUploadPart {
                    part: 1,
                    url: "u1".to_string(),
                    size_bytes: 2,
                },
                MultipartUploadPart {
                    part: 3,
                    url: "u3".to_string(),
                    size_bytes: 2,
                },
            ],
        }];

        let err = upload(&client, &source, &files).expect_err("part numbering must be contiguous");

        match err {
            UploadError::InvalidMultipart(msg) => assert!(msg.contains("expected 2, got 3")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn rejects_part_plan_exceeding_file_len() {
        let client = MockClient::default();
        let source = MockSource::new(HashMap::from([(
            "weights.bin".to_string(),
            b"abc".to_vec(),
        )]));
        let files = vec![MultipartUploadFile {
            rel_path: "weights.bin".to_string(),
            parts: vec![
                MultipartUploadPart {
                    part: 1,
                    url: "u1".to_string(),
                    size_bytes: 2,
                },
                MultipartUploadPart {
                    part: 2,
                    url: "u2".to_string(),
                    size_bytes: 2,
                },
            ],
        }];

        let err =
            upload(&client, &source, &files).expect_err("total part sizes cannot exceed file len");

        match err {
            UploadError::InvalidMultipart(msg) => assert!(msg.contains("exceeds file length")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn uploads_all_parts_with_expected_content() {
        let client = MockClient::default();
        let source = MockSource::new(HashMap::from([(
            "weights.bin".to_string(),
            b"abcdef".to_vec(),
        )]));
        let files = vec![MultipartUploadFile {
            rel_path: "weights.bin".to_string(),
            parts: vec![
                MultipartUploadPart {
                    part: 1,
                    url: "u1".to_string(),
                    size_bytes: 2,
                },
                MultipartUploadPart {
                    part: 2,
                    url: "u2".to_string(),
                    size_bytes: 2,
                },
                MultipartUploadPart {
                    part: 3,
                    url: "u3".to_string(),
                    size_bytes: 2,
                },
            ],
        }];

        upload(&client, &source, &files).expect("valid multipart plan should upload");

        let puts = client.puts.lock().expect("lock puts");
        assert_eq!(puts.len(), 3);
        assert_eq!(puts[0], ("u1".to_string(), 2, b"ab".to_vec()));
        assert_eq!(puts[1], ("u2".to_string(), 2, b"cd".to_vec()));
        assert_eq!(puts[2], ("u3".to_string(), 2, b"ef".to_vec()));
    }
}
