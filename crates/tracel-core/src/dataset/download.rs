use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use burn::data::dataset::Dataset;
use burn::data::dataset::DatasetError as BurnDatasetError;
use serde::de::DeserializeOwned;

use crate::{DatasetError, DatasetModule, DatasetVersionSpec, resolve_cache_dir};

/// Number of items requested per page while downloading a dataset to disk.
const DOWNLOAD_PAGE_SIZE: u32 = 500;

/// A Station dataset version, downloaded once to a local on-disk cache and adapted to
/// Burn's [`Dataset`] trait.
///
/// Unlike [`AnnotationDataset`](super::AnnotationDataset), which streams each item from
/// the backend on demand, `LocalAnnotationDataset` downloads the whole dataset version to
/// a local cache file the first time it's requested, then serves every subsequent
/// [`Dataset::get`] from that file with no further network access. Later requests for the
/// same name/version reuse the cached file as-is: presence on disk is the only check, no
/// integrity/hash verification is performed.
///
/// The cache file lives at `<platform cache dir>/datasets/<name>/<version>/items.jsonl`
/// (the platform cache dir is the one reported by the `directories` crate for
/// `ai.tracel.console`, e.g. `~/Library/Caches/ai.tracel.console` on macOS or
/// `~/.cache/burncentral` on Linux). If the local file has been corrupted, `get` surfaces
/// it as a [`DatasetError::CorruptCachedItem`], whose error message names the exact file
/// to delete before calling [`DatasetModule::download`] again to force a fresh download.
///
/// Build one with [`DatasetModule::download`] rather than constructing it directly; its
/// docs include a full usage example.
pub struct LocalAnnotationDataset<T> {
    name: String,
    version: u32,
    path: PathBuf,
    items: Vec<Vec<u8>>,
    _marker: std::marker::PhantomData<T>,
}

impl<T> LocalAnnotationDataset<T> {
    fn try_get_or_download(
        module: &DatasetModule,
        name: impl Into<String>,
        spec: DatasetVersionSpec,
        cache_root: &Path,
    ) -> Result<Self, DatasetError> {
        let name = name.into();
        let version = module.resolve_version(&name, spec)?;
        let path = cache_path(cache_root, &name, version);

        if !path.is_file() {
            download_to(module, &name, version, &path)?;
        }

        let items = read_items(&path)?;

        Ok(Self {
            name,
            version,
            path,
            items,
            _marker: std::marker::PhantomData,
        })
    }
}

impl<T> Dataset<T> for LocalAnnotationDataset<T>
where
    T: DeserializeOwned + Clone + Send + Sync,
{
    fn get(&self, index: usize) -> Result<T, BurnDatasetError> {
        let len = self.len();
        assert!(
            index < len,
            "Index out of bounds for LocalAnnotationDataset: {index} >= {len}"
        );

        let version = self.version;
        serde_json::from_slice::<T>(&self.items[index]).map_err(|err| {
            BurnDatasetError::new(DatasetError::CorruptCachedItem {
                name: self.name.clone(),
                version,
                index: index as u64,
                path: self.path.clone(),
                source: Box::new(err),
            })
        })
    }

    fn len(&self) -> usize {
        self.items.len()
    }
}

fn cache_path(root: &Path, name: &str, version: u32) -> PathBuf {
    root.join("datasets")
        .join(name)
        .join(version.to_string())
        .join("items.jsonl")
}

fn download_to(
    module: &DatasetModule,
    name: &str,
    version: u32,
    path: &Path,
) -> Result<(), DatasetError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("tmp");
    let mut writer = BufWriter::new(File::create(&tmp_path)?);

    let mut cursor: u64 = 0;
    loop {
        let page = module.get_items(name, version, Some(cursor), Some(DOWNLOAD_PAGE_SIZE))?;
        let page_len = page.items.len();

        for item in &page.items {
            writer.write_all(item)?;
            writer.write_all(b"\n")?;
        }

        if page_len < DOWNLOAD_PAGE_SIZE as usize {
            break;
        }
        cursor += page_len as u64;
    }

    writer.flush()?;
    drop(writer);
    fs::rename(&tmp_path, path)?;

    Ok(())
}

fn read_items(path: &Path) -> Result<Vec<Vec<u8>>, DatasetError> {
    let contents = fs::read(path)?;
    Ok(contents
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| line.to_vec())
        .collect())
}

impl DatasetModule {
    /// Adapts a named dataset version into a Burn [`Dataset`] backed by a local on-disk
    /// cache, downloading it once on first use.
    ///
    /// `spec` selects the version, same as [`DatasetModule::stream`]. Unlike that method,
    /// this one downloads the whole dataset version to a local cache file the first time
    /// it's requested, then serves items from disk with no further network access on later
    /// calls for the same name/version. See [`LocalAnnotationDataset`] for where that cache
    /// file lives and what to do if it's ever corrupted.
    ///
    /// ```rust,no_run
    /// use burn::data::dataloader::DataLoaderBuilder;
    /// use burn::data::dataloader::batcher::Batcher;
    /// use serde::Deserialize;
    /// use tracel_core::{DatasetModule, DatasetVersionSpec};
    ///
    /// #[derive(Deserialize, Clone, Debug)]
    /// struct Annotation {
    ///     label: String,
    /// }
    ///
    /// struct AnnotationBatcher;
    ///
    /// impl Batcher<Annotation, Vec<Annotation>> for AnnotationBatcher {
    ///     fn batch(&self, items: Vec<Annotation>, _device: &burn::tensor::Device) -> Vec<Annotation> {
    ///         items
    ///     }
    /// }
    ///
    /// # fn example(datasets: DatasetModule) -> Result<(), tracel_core::DatasetError> {
    /// // If the local cache is corrupted, the returned error already names the exact file
    /// // to delete before calling `download` again — see `DatasetError::CorruptCachedItem`.
    /// let dataset = datasets.download::<Annotation>("mnist-corrections", DatasetVersionSpec::Latest)?;
    ///
    /// let dataloader = DataLoaderBuilder::new(AnnotationBatcher)
    ///     .batch_size(32)
    ///     .build(dataset);
    /// # Ok(())
    /// # }
    /// ```
    pub fn download<T>(
        &self,
        name: impl Into<String>,
        spec: DatasetVersionSpec,
    ) -> Result<LocalAnnotationDataset<T>, DatasetError>
    where
        T: DeserializeOwned + Clone + Send + Sync,
    {
        let cache_root = resolve_cache_dir().ok_or(DatasetError::CacheUnavailable)?;
        LocalAnnotationDataset::try_get_or_download(self, name, spec, &cache_root)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use burn::data::dataset::Dataset;
    use serde::{Deserialize, Serialize};

    use super::{DOWNLOAD_PAGE_SIZE, LocalAnnotationDataset};
    use crate::dataset::{
        DatasetError, DatasetItemsPage, DatasetModule, DatasetProvider, DatasetVersionSpec,
    };

    #[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
    struct TestItem {
        value: u32,
    }

    struct FakeProvider<F> {
        stream: F,
    }

    impl<F> DatasetProvider for FakeProvider<F>
    where
        F: Fn(&str, u32, Option<u64>, Option<u32>) -> Result<DatasetItemsPage, DatasetError>
            + Send
            + Sync,
    {
        fn get_items(
            &self,
            name: &str,
            version: u32,
            index: Option<u64>,
            limit: Option<u32>,
        ) -> Result<DatasetItemsPage, DatasetError> {
            (self.stream)(name, version, index, limit)
        }

        fn item_count(&self, _name: &str, _version: u32) -> Result<u64, DatasetError> {
            Ok(0)
        }

        fn resolve_version(&self, _name: &str) -> Result<u32, DatasetError> {
            Ok(0)
        }
    }

    fn item(value: u32) -> Vec<u8> {
        serde_json::to_vec(&TestItem { value }).unwrap()
    }

    fn cache_file(root: &Path, name: &str, version: u32) -> PathBuf {
        root.join("datasets")
            .join(name)
            .join(version.to_string())
            .join("items.jsonl")
    }

    #[test]
    fn given_cache_miss_when_download_then_dataset_is_populated_from_paginated_provider() {
        let total = DOWNLOAD_PAGE_SIZE as usize + 150;
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let provider = FakeProvider {
            stream: move |_name: &str, _version: u32, cursor: Option<u64>, limit: Option<u32>| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                let start = cursor.unwrap_or(0) as usize;
                let limit = limit.map(|l| l as usize).unwrap_or(total);
                let end = (start + limit).min(total);
                Ok(DatasetItemsPage {
                    items: (start..end).map(|i| item(i as u32)).collect(),
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let cache_root = tempfile::tempdir().unwrap();

        let dataset: LocalAnnotationDataset<TestItem> =
            LocalAnnotationDataset::try_get_or_download(
                &module,
                "ds",
                DatasetVersionSpec::Fixed(1),
                cache_root.path(),
            )
            .unwrap();

        assert_eq!(dataset.len(), total);
        assert_eq!(dataset.get(0).unwrap(), TestItem { value: 0 });
        assert_eq!(
            dataset.get(total - 1).unwrap(),
            TestItem {
                value: (total - 1) as u32
            }
        );
        // One full page plus one shorter page to signal the end of the dataset.
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(cache_file(cache_root.path(), "ds", 1).is_file());
    }

    #[test]
    fn given_cache_hit_when_download_then_provider_is_not_called() {
        let cache_root = tempfile::tempdir().unwrap();
        let path = cache_file(cache_root.path(), "ds", 1);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, [item(1), item(2)].join(&b'\n').as_slice()).unwrap();

        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _cursor: Option<u64>, _limit: Option<u32>| {
                panic!("provider should not be called on a cache hit")
            },
        };
        let module = DatasetModule::new(Arc::new(provider));

        let dataset: LocalAnnotationDataset<TestItem> =
            LocalAnnotationDataset::try_get_or_download(
                &module,
                "ds",
                DatasetVersionSpec::Fixed(1),
                cache_root.path(),
            )
            .unwrap();

        assert_eq!(dataset.len(), 2);
        assert_eq!(dataset.get(0).unwrap(), TestItem { value: 1 });
        assert_eq!(dataset.get(1).unwrap(), TestItem { value: 2 });
    }

    #[test]
    fn given_corrupted_cache_file_when_get_then_corrupt_cached_item_error_names_the_path() {
        let cache_root = tempfile::tempdir().unwrap();
        let path = cache_file(cache_root.path(), "ds", 1);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not json\n").unwrap();

        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _cursor: Option<u64>, _limit: Option<u32>| {
                panic!("provider should not be called on a cache hit")
            },
        };
        let module = DatasetModule::new(Arc::new(provider));

        let dataset: LocalAnnotationDataset<TestItem> =
            LocalAnnotationDataset::try_get_or_download(
                &module,
                "ds",
                DatasetVersionSpec::Fixed(1),
                cache_root.path(),
            )
            .unwrap();

        let err = dataset.get(0).unwrap_err();
        let source = err
            .source()
            .expect("BurnDatasetError should carry the original DatasetError as its source");
        let dataset_err = source
            .downcast_ref::<DatasetError>()
            .expect("source should be our own DatasetError");

        match dataset_err {
            DatasetError::CorruptCachedItem {
                index,
                path: err_path,
                ..
            } => {
                assert_eq!(*index, 0);
                assert_eq!(err_path, &path);
            }
            other => panic!("expected CorruptCachedItem, got {other:?}"),
        }
    }

    #[test]
    fn given_provider_error_when_download_then_no_file_is_left_at_the_final_path() {
        let provider = FakeProvider {
            stream: |name: &str, _version: u32, _cursor: Option<u64>, _limit: Option<u32>| {
                Err(DatasetError::DatasetNotFound {
                    name: name.to_string(),
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let cache_root = tempfile::tempdir().unwrap();

        let result = LocalAnnotationDataset::<TestItem>::try_get_or_download(
            &module,
            "ds",
            DatasetVersionSpec::Fixed(1),
            cache_root.path(),
        );

        assert!(result.is_err());
        assert!(!cache_file(cache_root.path(), "ds", 1).exists());
    }
}
