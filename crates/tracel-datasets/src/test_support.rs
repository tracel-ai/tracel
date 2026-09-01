use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Deserialize;

use std::sync::Mutex;

use std::sync::Arc;

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

/// A publication that records what it is given.
struct FakePublication {
    batches: Arc<Mutex<Vec<Vec<NewItem>>>>,
    commits: Arc<Mutex<u32>>,
    cancels: Arc<Mutex<u32>>,
    pending: Vec<NewItem>,
    flush_every: usize,
    commit_fails: bool,
}

impl Publication for FakePublication {
    fn add_item(&mut self, item: NewItem) -> Result<(), DatasetsError> {
        self.pending.push(item);
        if self.pending.len() >= self.flush_every {
            self.batches
                .lock()
                .unwrap()
                .push(std::mem::take(&mut self.pending));
        }
        Ok(())
    }

    fn commit(
        &mut self,
        _metadata: Option<&serde_json::Value>,
    ) -> Result<DatasetVersion, DatasetsError> {
        if self.commit_fails {
            return Err(DatasetsError::other("the backend refused the commit"));
        }

        if !self.pending.is_empty() {
            self.batches
                .lock()
                .unwrap()
                .push(std::mem::take(&mut self.pending));
        }
        *self.commits.lock().unwrap() += 1;
        let count = self.batches.lock().unwrap().iter().flatten().count() as u64;
        Ok(version(count))
    }

    fn cancel(&mut self) -> Result<(), DatasetsError> {
        *self.cancels.lock().unwrap() += 1;
        self.pending.clear();
        Ok(())
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
    fn list_datasets(&self) -> Result<Vec<Dataset>, DatasetsError> {
        unimplemented!()
    }

    fn create_dataset(
        &self,
        name: &str,
        description: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<Dataset, DatasetsError> {
        Ok(Dataset {
            name: name.to_string(),
            description: description.map(str::to_string),
            metadata: metadata.cloned(),
        })
    }

    fn start_publication(&self, _dataset: &str) -> Result<Box<dyn Publication>, DatasetsError> {
        Ok(Box::new(FakePublication {
            batches: self.batches.clone(),
            commits: self.commits.clone(),
            cancels: self.cancels.clone(),
            pending: Vec::new(),
            flush_every: self.flush_every,
            commit_fails: self.commit_fails,
        }))
    }

    fn get_dataset(&self, _name: &str) -> Result<Dataset, DatasetsError> {
        unimplemented!()
    }

    fn list_versions(&self, _dataset: &str) -> Result<Vec<DatasetVersion>, DatasetsError> {
        unimplemented!()
    }

    fn get_version(
        &self,
        dataset: &str,
        spec: VersionSpec,
    ) -> Result<DatasetVersion, DatasetsError> {
        let mut resolved = version(self.item_count);
        resolved.dataset = dataset.to_string();
        if let VersionSpec::Exact(id) = spec {
            resolved.id = id;
        }
        Ok(resolved)
    }

    fn read_items(
        &self,
        _dataset: &str,
        _id: &VersionId,
        indexes: &[u64],
    ) -> Result<Vec<Item>, DatasetsError> {
        self.reads.fetch_add(1, Ordering::SeqCst);

        let answered = indexes.len().saturating_sub(self.short_by);
        Ok(indexes
            .iter()
            .take(answered)
            .map(|index| Item {
                example: index.to_string().into_bytes(),
                annotation: Some(self.annotation.clone()),
                source_item_id: Some(format!("source-{index}")),
                metadata: Some(serde_json::json!({ "split": "train" })),
            })
            .collect())
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
