use sha2::Digest;

use crate::api::{
    ArtifactFileSpecRequest, Client, ClientError, CreateArtifactRequest, MultipartUploadReponse,
};
use crate::schemas::ExperimentPath;

#[derive(Debug)]
pub struct TempLogStore {
    logs: Vec<String>,
    client: Client,
    experiment_path: ExperimentPath,
    bytes: usize,
}

impl TempLogStore {
    // 100 MiB per chunk
    const CHUNK_SIZE: usize = 100 * 1024 * 1024;

    pub fn new(client: Client, experiment_path: ExperimentPath) -> TempLogStore {
        TempLogStore {
            logs: Vec::new(),
            client,
            experiment_path,
            bytes: 0,
        }
    }

    pub fn push(&mut self, log: String) -> Result<(), ClientError> {
        self.bytes += log.len();
        self.logs.push(log);
        Ok(())
    }

    pub fn create_log_artifact(&mut self) -> Result<(), ClientError> {
        if self.logs.is_empty() {
            return Ok(());
        }

        let full_log = self.logs.join("");
        let log_bytes = full_log.as_bytes();

        // Split into chunks of max 100MB
        let chunks: Vec<&[u8]> = log_bytes.chunks(Self::CHUNK_SIZE).collect();
        let num_chunks = chunks.len();
        let num_digits = num_chunks.to_string().len();

        // Build file specs with checksums and sizes
        let mut file_specs = Vec::with_capacity(num_chunks);
        for (idx, chunk) in chunks.iter().enumerate() {
            let checksum = format!("{:x}", sha2::Sha256::new_with_prefix(chunk).finalize());
            let filename = format!("experiment-{:0width$}.log", idx, width = num_digits);

            file_specs.push(ArtifactFileSpecRequest {
                rel_path: filename,
                size_bytes: chunk.len() as u64,
                checksum,
            });
        }

        // Create artifact with all file specs
        let create_response = self.client.create_artifact(
            self.experiment_path.owner_name(),
            self.experiment_path.project_name(),
            self.experiment_path.experiment_num(),
            CreateArtifactRequest {
                name: "logs".to_string(),
                kind: "log".to_string(),
                files: file_specs,
            },
        )?;

        // Upload each chunk using multipart upload
        for (idx, chunk) in chunks.iter().enumerate() {
            let filename = format!("experiment-{:0width$}.log", idx, width = num_digits);

            // Find the corresponding upload URLs
            let file_response = create_response
                .files
                .iter()
                .find(|f| f.rel_path == filename)
                .ok_or_else(|| {
                    ClientError::UnknownError(format!(
                        "Missing upload URL for log file: {}",
                        filename
                    ))
                })?;

            // Upload the chunk using multipart upload
            self.upload_chunk_multipart(chunk, &file_response.urls)?;
        }

        // Complete the artifact upload
        self.client.complete_artifact_upload(
            self.experiment_path.owner_name(),
            self.experiment_path.project_name(),
            self.experiment_path.experiment_num(),
            &create_response.id,
        )?;

        // Clear the log buffer
        self.logs.clear();
        self.bytes = 0;

        Ok(())
    }

    fn upload_chunk_multipart(
        &self,
        chunk_data: &[u8],
        multipart_info: &MultipartUploadReponse,
    ) -> Result<(), ClientError> {
        // Sort parts by part number
        let mut part_indices: Vec<usize> = (0..multipart_info.parts.len()).collect();
        part_indices.sort_by_key(|&i| multipart_info.parts[i].part);

        // Validate part numbering
        for (i, &part_idx) in part_indices.iter().enumerate() {
            let part = &multipart_info.parts[part_idx];
            if part.part != (i as u32 + 1) {
                return Err(ClientError::UnknownError(format!(
                    "Invalid part numbering: expected part {}, got part {}",
                    i + 1,
                    part.part
                )));
            }
        }

        // Upload each part
        let mut current_offset = 0usize;
        for &part_idx in part_indices.iter() {
            let part_info = &multipart_info.parts[part_idx];
            let end_offset = std::cmp::min(
                current_offset + part_info.size_bytes as usize,
                chunk_data.len(),
            );

            if current_offset >= chunk_data.len() {
                break;
            }

            let part_data = &chunk_data[current_offset..end_offset];
            self.client
                .upload_bytes_to_url(&part_info.url, part_data.to_vec())?;

            current_offset = end_offset;
        }

        Ok(())
    }
}
