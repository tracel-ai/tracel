use std::collections::HashMap;
use std::future::Future;
use std::ops::Range;
use std::sync::Arc;

use crate::error::client_error_is_not_found;
use tracel_client::console::Client;
use tracel_client::console::dataset::request::{
    AddDatasetVersionUploadItemsRequest, CompleteDatasetVersionUploadRequest, CreateDatasetRequest,
    DatasetVersionUploadItemRequest, QueryDatasetVersionsRequest, QueryDatasetsRequest,
};
use tracel_client::console::dataset::response::{DatasetResponse, DatasetVersionResponse};
use tracel_datasets::{
    Dataset, DatasetOps, DatasetVersion, DatasetsError, Item, NewItem, Publication, VersionId,
    VersionSpec,
};
use tracel_task::{Job, MaybeSend};

use crate::ConsoleError;
use crate::console::ProjectScope;
use crate::wire::console_timestamp;

/// What one item-upload request is allowed to reach. The console rejects a larger body, so a
/// batch is sent before it can get there rather than after the console refuses it.
const BATCH_BYTES: usize = 63 * 1024 * 1024;
/// How many datasets or versions to fetch in one console query.
const PAGE_SIZE: u32 = 100;

#[derive(Clone)]
pub struct ConsoleDatasetOps {
    pub scope: Arc<ProjectScope>,
}

impl ConsoleDatasetOps {
    async fn versions(&self, dataset: &str) -> Result<Vec<DatasetVersion>, DatasetsError> {
        let scope = &self.scope;
        let versions = collect_pages(|page| async move {
            scope
                .console
                .client
                .query_dataset_versions(
                    &scope.owner,
                    &scope.project,
                    dataset,
                    QueryDatasetVersionsRequest {
                        page: Some(page),
                        per_page: Some(PAGE_SIZE),
                    },
                )
                .await
                .map(|response| (response.items, response.total_count))
                .map_err(|error| map_dataset_error(error, dataset))
        })
        .await?;

        Ok(versions
            .into_iter()
            .map(|version| version_from_wire(dataset, version))
            .collect())
    }

    async fn items(
        &self,
        dataset: &str,
        id: &VersionId,
        indexes: &[u64],
    ) -> Result<Vec<Item>, DatasetsError> {
        let scope = &self.scope;
        let version = route_version(dataset, id)?;
        let mut read = HashMap::with_capacity(indexes.len());

        for run in contiguous_runs(indexes) {
            let mut next = run.start;
            while next < run.end {
                let page = scope
                    .console
                    .client
                    .stream_dataset_version_items(
                        &scope.owner,
                        &scope.project,
                        dataset,
                        version,
                        Some(next),
                        Some((run.end - next).min(u32::MAX as u64) as u32),
                    )
                    .await
                    .map_err(|error| map_version_error(error, dataset, id))?;

                if page.items.is_empty() {
                    break;
                }

                let asked_from = next;
                for item in page.items {
                    next = item.entry_idx + 1;
                    if item.entry_idx < run.end {
                        read.insert(item.entry_idx, item_from_wire(&item.payload)?);
                    }
                }

                // A page that leaves the cursor where it was would be asked for forever.
                if next <= asked_from {
                    break;
                }
            }
        }

        let found = read.len() as u64;
        ordered_items(indexes, &read).ok_or(DatasetsError::Incomplete {
            dataset: dataset.to_string(),
            version: id.clone(),
            expected: indexes.len() as u64,
            actual: found,
        })
    }
}

impl DatasetOps for ConsoleDatasetOps {
    fn list_datasets(&self) -> Job<Vec<Dataset>, DatasetsError> {
        let this = self.clone();
        self.scope.console.attach(async move {
            let scope = &this.scope;
            let datasets = collect_pages(|page| async move {
                scope
                    .console
                    .client
                    .query_datasets(
                        &scope.owner,
                        &scope.project,
                        QueryDatasetsRequest {
                            page: Some(page),
                            per_page: Some(PAGE_SIZE),
                        },
                    )
                    .await
                    .map(|response| (response.items, response.total_count))
                    .map_err(console_failure)
            })
            .await?;

            Ok(datasets.into_iter().map(dataset_from_wire).collect())
        })
    }

    fn get_dataset(&self, name: String) -> Job<Dataset, DatasetsError> {
        let this = self.clone();
        self.scope.console.attach(async move {
            let scope = &this.scope;
            scope
                .console
                .client
                .get_dataset(&scope.owner, &scope.project, &name)
                .await
                .map(dataset_from_wire)
                .map_err(|error| map_dataset_error(error, &name))
        })
    }

    fn list_versions(&self, dataset: String) -> Job<Vec<DatasetVersion>, DatasetsError> {
        let this = self.clone();
        self.scope
            .console
            .attach(async move { this.versions(&dataset).await })
    }

    fn get_version(
        &self,
        dataset: String,
        spec: VersionSpec,
    ) -> Job<DatasetVersion, DatasetsError> {
        let this = self.clone();
        self.scope.console.attach(async move {
            let versions = this.versions(&dataset).await?;
            let found = match &spec {
                VersionSpec::Exact(wanted) => {
                    versions.into_iter().find(|version| &version.id == wanted)
                }
                VersionSpec::Latest => versions.into_iter().max_by_key(|version| version.version),
            };

            found.ok_or(DatasetsError::VersionNotFound {
                dataset,
                version: spec,
            })
        })
    }

    fn create_dataset(
        &self,
        name: String,
        description: Option<String>,
        metadata: Option<serde_json::Value>,
    ) -> Job<Dataset, DatasetsError> {
        let this = self.clone();
        self.scope.console.attach(async move {
            let scope = &this.scope;
            scope
                .console
                .client
                .create_dataset(
                    &scope.owner,
                    &scope.project,
                    CreateDatasetRequest {
                        name,
                        description,
                        metadata,
                    },
                )
                .await
                .map(dataset_from_wire)
                .map_err(console_failure)
        })
    }

    fn start_publication(&self, dataset: String) -> Job<Box<dyn Publication>, DatasetsError> {
        let this = self.clone();
        self.scope.console.attach(async move {
            let scope = &this.scope;
            let started = scope
                .console
                .client
                .start_dataset_version_upload(&scope.owner, &scope.project, &dataset)
                .await
                .map_err(|error| map_dataset_error(error, &dataset))?;

            Ok(Box::new(ConsolePublication {
                scope: Arc::clone(&this.scope),
                dataset,
                upload_id: started.upload_id,
                pending: Vec::new(),
                pending_bytes: 0,
                settled: false,
            }) as Box<dyn Publication>)
        })
    }

    fn read_items(
        &self,
        dataset: String,
        id: VersionId,
        indexes: Vec<u64>,
    ) -> Job<Vec<Item>, DatasetsError> {
        let this = self.clone();
        self.scope
            .console
            .attach(async move { this.items(&dataset, &id, &indexes).await })
    }
}

/// One upload, sending items in batches.
///
/// Dropped before it is committed or cancelled, it asks the console to cancel the upload on the
/// connection's runtime, without waiting for the answer.
struct ConsolePublication {
    scope: Arc<ProjectScope>,
    dataset: String,
    upload_id: String,
    pending: Vec<DatasetVersionUploadItemRequest>,
    pending_bytes: usize,
    settled: bool,
}

impl ConsolePublication {
    /// Hands back the upload of everything held, or nothing when nothing is held.
    fn upload(&mut self) -> Option<Job<(), DatasetsError>> {
        if self.pending.is_empty() {
            return None;
        }

        let items = std::mem::take(&mut self.pending);
        self.pending_bytes = 0;
        let scope = Arc::clone(&self.scope);
        let dataset = self.dataset.clone();
        let upload_id = self.upload_id.clone();
        Some(self.scope.console.attach(async move {
            scope
                .console
                .client
                .add_dataset_version_upload_items(
                    &scope.owner,
                    &scope.project,
                    &dataset,
                    &upload_id,
                    AddDatasetVersionUploadItemsRequest { items },
                )
                .await
                .map(drop)
                .map_err(console_failure)
        }))
    }

    /// Asks the console to cancel the upload, owning everything the request needs.
    fn cancel_request(
        &self,
    ) -> impl Future<Output = Result<(), DatasetsError>> + MaybeSend + 'static {
        let client: Client = self.scope.console.client.clone();
        let owner = self.scope.owner.clone();
        let project = self.scope.project.clone();
        let dataset = self.dataset.clone();
        let upload_id = self.upload_id.clone();
        async move {
            client
                .cancel_dataset_version_upload(&owner, &project, &dataset, &upload_id)
                .await
                .map_err(console_failure)
        }
    }
}

impl Publication for ConsolePublication {
    fn add_item(&mut self, item: NewItem) -> Result<Option<Job<(), DatasetsError>>, DatasetsError> {
        let item = DatasetVersionUploadItemRequest {
            source_item_id: item.source_item_id,
            example_payload: item.example,
            annotation: item.annotation,
            metadata: item.metadata,
        };
        let size = encoded_size(&item);

        if size > BATCH_BYTES {
            return Err(DatasetsError::other(format!(
                "one item encodes to {size} bytes, more than the {BATCH_BYTES} an upload holds"
            )));
        }

        let upload = (self.pending_bytes + size > BATCH_BYTES)
            .then(|| self.upload())
            .flatten();
        self.pending_bytes += size;
        self.pending.push(item);

        Ok(upload)
    }

    fn commit(
        mut self: Box<Self>,
        metadata: Option<serde_json::Value>,
    ) -> Job<DatasetVersion, DatasetsError> {
        self.settled = true;
        let upload = self.upload();
        let scope = Arc::clone(&self.scope);
        let dataset = self.dataset.clone();
        let upload_id = self.upload_id.clone();
        self.scope.console.attach(async move {
            if let Some(upload) = upload {
                upload.await?;
            }
            scope
                .console
                .client
                .complete_dataset_version_upload(
                    &scope.owner,
                    &scope.project,
                    &dataset,
                    &upload_id,
                    CompleteDatasetVersionUploadRequest { metadata },
                )
                .await
                .map(|version| version_from_wire(&dataset, version))
                .map_err(console_failure)
        })
    }

    fn cancel(mut self: Box<Self>) -> Job<(), DatasetsError> {
        self.settled = true;
        self.scope.console.attach(self.cancel_request())
    }
}

impl Drop for ConsolePublication {
    fn drop(&mut self) {
        if !self.settled {
            let cancel = self.cancel_request();
            self.scope.console.runtime.spawn(async move {
                let _ = cancel.await;
            });
        }
    }
}

/// The bytes `item` adds to a request body.
///
/// The payload travels base64-encoded, so it is a third larger on the wire than in memory.
fn encoded_size(item: &DatasetVersionUploadItemRequest) -> usize {
    /// Field names, quotes, braces and commas around one item.
    const ENVELOPE: usize = 96;

    // Everything but the payload is measured serialized rather than guessed at: JSON escaping
    // makes a quote two bytes and a control character six, so the wire form of a string can be
    // far longer than the Rust one.
    ENVELOPE
        + item.example_payload.len().div_ceil(3) * 4
        + json_len(&item.source_item_id)
        + json_len(&item.annotation)
        + json_len(&item.metadata)
}

/// The bytes `value` serializes to, without keeping them.
///
/// A value that cannot be serialized is reported as too large for any batch, so it is refused
/// rather than sent and rejected.
fn json_len(value: &impl serde::Serialize) -> usize {
    struct Counter(usize);

    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut counter = Counter(0);
    match serde_json::to_writer(&mut counter, value) {
        Ok(()) => counter.0,
        Err(_) => usize::MAX,
    }
}

/// The document a streamed item carries, shared by every backend the console serves.
#[serde_with::serde_as]
#[derive(serde::Deserialize)]
struct WireItem {
    source_item_id: Option<String>,
    metadata: Option<serde_json::Value>,
    #[serde_as(as = "serde_with::base64::Base64")]
    example_payload: Vec<u8>,
    annotation: Option<serde_json::Value>,
}

/// The ascending, contiguous stretches `indexes` covers, so each is one request.
fn contiguous_runs(indexes: &[u64]) -> Vec<Range<u64>> {
    let mut sorted: Vec<u64> = indexes.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut runs: Vec<Range<u64>> = Vec::new();
    for index in sorted {
        match runs.last_mut() {
            Some(run) if run.end == index => run.end = index + 1,
            _ => runs.push(index..index + 1),
        }
    }

    runs
}

/// Reorders unique items into the caller's requested order, retaining repetitions.
fn ordered_items(indexes: &[u64], items: &HashMap<u64, Item>) -> Option<Vec<Item>> {
    indexes
        .iter()
        .map(|index| items.get(index).cloned())
        .collect()
}

/// Reads every page of one console query.
async fn collect_pages<T, F, Fut>(mut fetch: F) -> Result<Vec<T>, DatasetsError>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<(Vec<T>, u64), DatasetsError>>,
{
    let mut all = Vec::new();
    let mut page = 0;

    loop {
        let (items, total) = fetch(page).await?;
        let count = items.len();
        all.extend(items);

        if all.len() as u64 >= total || count < PAGE_SIZE as usize || page == u32::MAX {
            return Ok(all);
        }

        page += 1;
    }
}

fn item_from_wire(payload: &[u8]) -> Result<Item, DatasetsError> {
    let wire: WireItem = serde_json::from_slice(payload)
        .map_err(|error| DatasetsError::other(ConsoleError::InvalidResponse(error.to_string())))?;

    Ok(Item {
        example: wire.example_payload,
        annotation: wire.annotation,
        source_item_id: wire.source_item_id,
        metadata: wire.metadata,
    })
}

fn route_version(dataset: &str, id: &VersionId) -> Result<u32, DatasetsError> {
    id.as_str()
        .parse()
        .map_err(|_| DatasetsError::VersionNotFound {
            dataset: dataset.to_string(),
            version: VersionSpec::Exact(id.clone()),
        })
}

fn dataset_from_wire(response: DatasetResponse) -> Dataset {
    Dataset {
        name: response.name,
        description: response.description,
        metadata: response.metadata,
    }
}

fn version_from_wire(dataset: &str, response: DatasetVersionResponse) -> DatasetVersion {
    DatasetVersion {
        dataset: dataset.to_string(),
        id: VersionId::new(response.version.max(0).to_string()),
        version: Some(response.version.max(0) as u32),
        item_count: response.item_count,
        metadata: response.metadata,
        created_at: console_timestamp(&response.created_at),
    }
}

fn console_failure(error: tracel_client::ClientError) -> DatasetsError {
    DatasetsError::other(ConsoleError::from(error))
}

/// Reads a client failure as the dataset problem it stands for.
fn map_dataset_error(error: tracel_client::ClientError, dataset: &str) -> DatasetsError {
    if client_error_is_not_found(&error) {
        return DatasetsError::DatasetNotFound {
            name: dataset.to_string(),
        };
    }
    console_failure(error)
}

fn map_version_error(
    error: tracel_client::ClientError,
    dataset: &str,
    id: &VersionId,
) -> DatasetsError {
    if client_error_is_not_found(&error) {
        return DatasetsError::VersionNotFound {
            dataset: dataset.to_string(),
            version: VersionSpec::Exact(id.clone()),
        };
    }
    console_failure(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_batch_of_neighbours_is_one_run() {
        assert_eq!(contiguous_runs(&[4, 5, 6]), vec![4..7]);
    }

    #[test]
    fn a_shuffled_batch_is_sorted_and_coalesced() {
        assert_eq!(contiguous_runs(&[9, 1, 8, 0, 2]), vec![0..3, 8..10]);
    }

    #[test]
    fn a_repeated_index_is_asked_for_once() {
        assert_eq!(contiguous_runs(&[3, 3, 3]), vec![3..4]);
    }

    #[test]
    fn the_estimate_never_undercounts_what_is_actually_sent() {
        let payloads = [0usize, 1, 2, 3, 4, 5, 1_000, 1_001];
        let ids = [
            None,
            Some(String::new()),
            Some("plain".to_string()),
            // JSON escapes make the wire form longer than the Rust string.
            Some("\"quoted\"".to_string()),
            Some("back\\slash".to_string()),
            Some("tab\there\nand\nnewlines".to_string()),
            Some("\u{1}\u{2}\u{3} control".to_string()),
            Some("emoji \u{1F600} and accents \u{e9}\u{e8}".to_string()),
            Some("\"".repeat(64)),
        ];
        let extras = [
            None,
            Some(serde_json::json!(null)),
            Some(serde_json::json!({"label": "cat", "nested": [1, 2, 3]})),
            Some(serde_json::json!({"quote": "say \"hi\"", "uni": "\u{e9}"})),
        ];

        for payload in payloads {
            for id in &ids {
                for extra in &extras {
                    let item = DatasetVersionUploadItemRequest {
                        source_item_id: id.clone(),
                        example_payload: vec![7; payload],
                        annotation: extra.clone(),
                        metadata: extra.clone(),
                    };

                    let actual = serde_json::to_vec(&item).expect("an item serializes").len();
                    let estimate = encoded_size(&item);

                    assert!(
                        estimate >= actual,
                        "undercounted by {}: payload={payload} id={id:?} extra={extra:?}",
                        actual - estimate
                    );
                }
            }
        }
    }

    #[test]
    fn the_budget_bounds_the_whole_request_body_not_just_the_items() {
        let items: Vec<_> = (0..500)
            .map(|n| DatasetVersionUploadItemRequest {
                source_item_id: Some(format!("item-\"{n}\"")),
                example_payload: vec![n as u8; n * 7 % 1_000],
                annotation: Some(serde_json::json!({ "label": n })),
                metadata: None,
            })
            .collect();

        let budgeted: usize = items.iter().map(encoded_size).sum();
        let request = AddDatasetVersionUploadItemsRequest {
            items: items.clone(),
        };
        let actual = serde_json::to_vec(&request)
            .expect("a batch serializes")
            .len();

        assert!(
            budgeted >= actual,
            "a batch of {} items budgeted {budgeted} but sends {actual}",
            items.len()
        );
    }

    #[test]
    fn a_payload_is_measured_base64_encoded_not_raw() {
        let item = DatasetVersionUploadItemRequest {
            source_item_id: None,
            example_payload: vec![0; 3_000],
            annotation: None,
            metadata: None,
        };

        // 3 raw bytes become 4 on the wire, so a batch budgeted on raw sizes overshoots.
        assert_eq!(encoded_size(&item) - 96 - 4 - 4 - 4, 4_000);
    }

    #[test]
    fn a_payload_that_alone_exceeds_the_budget_is_refused() {
        let raw = BATCH_BYTES / 4 * 3 + 1;
        let item = DatasetVersionUploadItemRequest {
            source_item_id: None,
            example_payload: vec![0; raw],
            annotation: None,
            metadata: None,
        };

        assert!(encoded_size(&item) > BATCH_BYTES);
    }

    #[test]
    fn a_megabyte_item_fills_the_budget_long_before_a_large_count_does() {
        let item = DatasetVersionUploadItemRequest {
            source_item_id: None,
            example_payload: vec![0; 1024 * 1024],
            annotation: None,
            metadata: None,
        };

        assert!(BATCH_BYTES / encoded_size(&item) < 64);
    }

    #[test]
    fn small_items_are_not_held_back_by_a_count() {
        let item = DatasetVersionUploadItemRequest {
            source_item_id: None,
            example_payload: vec![0; 1024],
            annotation: None,
            metadata: None,
        };

        // Bytes alone decide, so a batch holds far more than the 256 it used to.
        assert!(BATCH_BYTES / encoded_size(&item) > 10_000);
    }

    #[test]
    fn a_repeated_index_is_returned_each_time() {
        let item = Item {
            example: b"three".to_vec(),
            annotation: None,
            source_item_id: None,
            metadata: None,
        };
        let items = HashMap::from([(3, item.clone())]);

        assert_eq!(
            ordered_items(&[3, 3], &items),
            Some(vec![item.clone(), item])
        );
    }

    #[test]
    fn queries_every_page_needed_to_reach_the_total() {
        let mut fetched = Vec::new();
        let mut pages = vec![
            ((0..PAGE_SIZE).collect::<Vec<_>>(), u64::from(PAGE_SIZE) + 1),
            (vec![PAGE_SIZE], u64::from(PAGE_SIZE) + 1),
        ]
        .into_iter();

        let values = futures::executor::block_on(collect_pages(|page| {
            fetched.push(page);
            std::future::ready(Ok(pages.next().expect("the test provided this page")))
        }))
        .unwrap();

        assert_eq!(fetched, [0, 1]);
        assert_eq!(values.len(), PAGE_SIZE as usize + 1);
    }
}
