use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use tracel_task::Job;

use crate::{
    Dataset, DatasetOps, DatasetVersion, DatasetsError, Item, NewItem, Publication, VersionId,
    VersionSpec,
};

/// An annotation type for tests that decode one.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Label {
    pub value: u32,
}

/// A backend that answers from nothing, counting how often it is asked.
pub struct FakeOps {
    reads: AtomicUsize,
    /// How many items the version this backend resolves holds.
    pub item_count: u64,
    /// How many fewer items than asked for to answer with.
    pub short_by: usize,
    /// The annotation every item carries.
    pub annotation: serde_json::Value,
    /// Batches the backend received, in order.
    pub batches: Arc<Mutex<Vec<Vec<NewItem>>>>,
    /// Versions the backend committed.
    pub commits: Arc<Mutex<u32>>,
    /// How many items this backend holds before sending a batch.
    pub flush_every: usize,
    /// Publications that were cancelled.
    pub cancels: Arc<Mutex<u32>>,
    /// Whether committing fails.
    pub commit_fails: bool,
}

impl FakeOps {
    /// Every item the backend received, flattened.
    pub fn received(&self) -> Vec<NewItem> {
        self.batches
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .cloned()
            .collect()
    }

    /// How many batches the backend received.
    pub fn batch_count(&self) -> usize {
        self.batches.lock().unwrap().len()
    }

    /// How many publications were committed.
    pub fn commits(&self) -> u32 {
        *self.commits.lock().unwrap()
    }

    /// How many publications were cancelled.
    pub fn cancels(&self) -> u32 {
        *self.cancels.lock().unwrap()
    }
}

/// A publication that records what it is given; a batch travels only when its upload is driven.
struct FakePublication {
    batches: Arc<Mutex<Vec<Vec<NewItem>>>>,
    commits: Arc<Mutex<u32>>,
    cancels: Arc<Mutex<u32>>,
    pending: Vec<NewItem>,
    flush_every: usize,
    commit_fails: bool,
    settled: bool,
}

impl FakePublication {
    fn upload(&mut self) -> Job<(), DatasetsError> {
        let batch = std::mem::take(&mut self.pending);
        let batches = Arc::clone(&self.batches);
        Job::new(async move {
            batches.lock().unwrap().push(batch);
            Ok(())
        })
    }
}

impl Publication for FakePublication {
    fn add_item(&mut self, item: NewItem) -> Result<Option<Job<(), DatasetsError>>, DatasetsError> {
        self.pending.push(item);
        Ok((self.pending.len() >= self.flush_every).then(|| self.upload()))
    }

    fn commit(
        mut self: Box<Self>,
        _metadata: Option<serde_json::Value>,
    ) -> Job<DatasetVersion, DatasetsError> {
        self.settled = true;
        if self.commit_fails {
            return Job::failed(DatasetsError::other("the backend refused the commit"));
        }

        let upload = (!self.pending.is_empty()).then(|| self.upload());
        let commits = Arc::clone(&self.commits);
        let batches = Arc::clone(&self.batches);
        Job::new(async move {
            if let Some(upload) = upload {
                upload.await?;
            }
            *commits.lock().unwrap() += 1;
            let count = batches.lock().unwrap().iter().flatten().count() as u64;
            Ok(version(count))
        })
    }

    fn cancel(mut self: Box<Self>) -> Job<(), DatasetsError> {
        self.settled = true;
        *self.cancels.lock().unwrap() += 1;
        Job::ready(())
    }
}

impl Drop for FakePublication {
    fn drop(&mut self) {
        if !self.settled {
            *self.cancels.lock().unwrap() += 1;
        }
    }
}

impl FakeOps {
    pub fn new() -> Self {
        Self {
            reads: AtomicUsize::new(0),
            item_count: 10,
            short_by: 0,
            annotation: serde_json::json!({ "value": 1 }),
            batches: Arc::new(Mutex::new(Vec::new())),
            commits: Arc::new(Mutex::new(0)),
            flush_every: 256,
            cancels: Arc::new(Mutex::new(0)),
            commit_fails: false,
        }
    }

    /// How many times the backend was asked for items.
    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl DatasetOps for FakeOps {
    fn list_datasets(&self) -> Job<Vec<Dataset>, DatasetsError> {
        unimplemented!()
    }

    fn create_dataset(
        &self,
        name: String,
        description: Option<String>,
        metadata: Option<serde_json::Value>,
    ) -> Job<Dataset, DatasetsError> {
        Job::ready(Dataset {
            name,
            description,
            metadata,
        })
    }

    fn start_publication(&self, _dataset: String) -> Job<Box<dyn Publication>, DatasetsError> {
        Job::ready(Box::new(FakePublication {
            batches: self.batches.clone(),
            commits: self.commits.clone(),
            cancels: self.cancels.clone(),
            pending: Vec::new(),
            flush_every: self.flush_every,
            commit_fails: self.commit_fails,
            settled: false,
        }))
    }

    fn get_dataset(&self, _name: String) -> Job<Dataset, DatasetsError> {
        unimplemented!()
    }

    fn list_versions(&self, _dataset: String) -> Job<Vec<DatasetVersion>, DatasetsError> {
        unimplemented!()
    }

    fn get_version(
        &self,
        dataset: String,
        spec: VersionSpec,
    ) -> Job<DatasetVersion, DatasetsError> {
        let mut resolved = version(self.item_count);
        resolved.dataset = dataset;
        if let VersionSpec::Exact(id) = spec {
            resolved.id = id;
        }
        Job::ready(resolved)
    }

    fn read_items(
        &self,
        _dataset: String,
        _id: VersionId,
        indexes: Vec<u64>,
    ) -> Job<Vec<Item>, DatasetsError> {
        self.reads.fetch_add(1, Ordering::SeqCst);

        let answered = indexes.len().saturating_sub(self.short_by);
        Job::ready(
            indexes
                .iter()
                .take(answered)
                .map(|index| Item {
                    example: index.to_string().into_bytes(),
                    annotation: Some(self.annotation.clone()),
                    source_item_id: Some(format!("source-{index}")),
                    metadata: Some(serde_json::json!({ "split": "train" })),
                })
                .collect(),
        )
    }
}

/// A version holding `item_count` items.
pub fn version(item_count: u64) -> DatasetVersion {
    DatasetVersion {
        dataset: "ds".to_string(),
        id: VersionId::new("v1"),
        version: Some(1),
        item_count,
        created_at: None,
        metadata: None,
    }
}
