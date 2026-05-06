use burn::data::dataset::{Dataset, DatasetIterator};
use burn_central_client::{StationClient, station::dataset::StreamDatasetVersionItemsRequest};
use serde::Deserialize;

pub struct DatasetRef {
    pub name: String,
    pub version: u32,
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

#[derive(Debug, Clone, Deserialize)]
#[serde_with::serde_as]
pub struct AnnotationItem {
    pub source_item_id: String,
    #[serde_as(as = "serde_with::base64::Base64")]
    pub example_payload: Vec<u8>,
    pub example_size_bytes: u64,
    pub annotation: serde_json::Value,
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

        let items = items
            .items
            .into_iter()
            .map(|payload| serde_json::from_slice(&payload.payload).ok())
            .collect::<Option<Vec<AnnotationItem>>>()?;

        items.into_iter().next()
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
