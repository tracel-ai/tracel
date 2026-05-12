use burn::data::dataset::{Dataset, DatasetIterator};
use burn_central_client::{StationClient, station::dataset::StreamDatasetVersionItemsRequest};
use serde::Deserialize;

pub struct DatasetRef {
    pub name: String,
    pub version: u32,
}

impl DatasetRef {
    pub fn new(name: String, version: u32) -> Self {
        Self { name, version }
    }
}

pub struct AnnotationDataset {
    client: StationClient,
    dataset_ref: DatasetRef,
}

impl AnnotationDataset {
    pub fn new(client: StationClient, dataset_ref: DatasetRef) -> Self {
        Self {
            client,
            dataset_ref,
        }
    }
}

/// Specific item serialization format for a dataset of type "annotation_set".
#[serde_with::serde_as]
#[derive(Debug, Clone, Deserialize)]
pub struct AnnotationItem {
    pub source_item_id: Option<String>,
    #[serde_as(as = "serde_with::base64::Base64")]
    pub example_payload: Vec<u8>,
    pub example_size_bytes: u64,
    pub annotation: Option<serde_json::Value>,
}

impl Dataset<AnnotationItem> for AnnotationDataset {
    fn get(&self, index: usize) -> Option<AnnotationItem> {
        let items = self
            .client
            .datasets()
            .stream_items(
                &self.dataset_ref.name,
                self.dataset_ref.version,
                StreamDatasetVersionItemsRequest {
                    cursor: Some(index as u64),
                    limit: Some(1),
                },
            )
            .ok()?;

        let mut errors = String::new();

        let item = items
            .items
            .into_iter()
            .find(|item| item.entry_idx == index as u64)?;

        match serde_json::from_slice::<AnnotationItem>(&item.payload) {
            Ok(item) => return Some(item),
            Err(e) => errors.push_str(&format!("Failed to parse item: {}\n", e)),
        }

        if !errors.is_empty() {
            eprintln!("Errors occurred while parsing items:\n{}", errors);
        }

        None
    }

    fn len(&self) -> usize {
        let mut count = 0;
        let mut cursor = None;

        loop {
            let Some(response) = self
                .client
                .datasets()
                .stream_items(
                    &self.dataset_ref.name,
                    self.dataset_ref.version,
                    StreamDatasetVersionItemsRequest {
                        cursor,
                        limit: Some(1000),
                    },
                )
                .ok()
            else {
                break;
            };

            count += response.items.len();
            if let Some(next_cursor) = response.next_cursor {
                cursor = Some(next_cursor);
            } else {
                break;
            }
        }
        count
    }

    fn is_empty(&self) -> bool {
        self.get(0).is_none()
    }

    fn iter(&self) -> DatasetIterator<'_, AnnotationItem>
    where
        Self: Sized,
    {
        DatasetIterator::new(self)
    }
}
