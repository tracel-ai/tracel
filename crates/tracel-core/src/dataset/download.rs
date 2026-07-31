use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use burn::data::dataset::{Dataset, DatasetError as BurnDatasetError};
use serde::de::DeserializeOwned;

use super::item::{ItemLocation, decode_item};
use super::{DatasetError, DatasetItem, DatasetProvider};

/// Number of items requested per page while downloading a dataset to disk.
const DOWNLOAD_PAGE_SIZE: u32 = 500;

/// A Station dataset version downloaded into a local cache before it is used.
///
/// Unlike [`StreamedDataset`](super::StreamedDataset), which requests an item from the backend
/// on every access, `DownloadedDataset` downloads all raw item envelopes on a cache miss. At
/// construction, the cache file is read into memory once; [`Dataset::get`] then decodes an item
/// from those in-memory bytes without further file or network access. Later constructions for the
/// same name and version reuse the cache file as-is: presence on disk is the only check, with no
/// integrity or hash verification.
///
/// The cache file lives at `<platform cache dir>/datasets/<name>/<version>/items.jsonl` (the
/// platform cache directory reported for `tracel`, such as `~/Library/Caches/tracel` on macOS or
/// `~/.cache/tracel` on Linux). If the local file has been corrupted, `get` returns
/// [`DatasetError::CorruptCachedItem`], whose message names the exact file to delete before
/// calling [`DatasetModule::download`](super::DatasetModule::download) again.
pub struct DownloadedDataset<A> {
    _provider: Arc<dyn DatasetProvider>,
    name: String,
    version: u32,
    path: PathBuf,
    items: Vec<Vec<u8>>,
    _marker: std::marker::PhantomData<A>,
}

impl<A> DownloadedDataset<A> {
    pub(super) fn try_get_or_download(
        provider: Arc<dyn DatasetProvider>,
        name: String,
        version: u32,
        cache_root: &Path,
    ) -> Result<Self, DatasetError> {
        let path = cache_path(cache_root, &name, version);

        if !path.is_file() {
            download_to(provider.as_ref(), &name, version, &path)?;
        }

        let items = read_items(&path)?;

        Ok(Self {
            _provider: provider,
            name,
            version,
            path,
            items,
            _marker: std::marker::PhantomData,
        })
    }
}

impl<A> Dataset<DatasetItem<A>> for DownloadedDataset<A>
where
    A: DeserializeOwned + Clone + Send + Sync,
{
    fn get(&self, index: usize) -> Result<DatasetItem<A>, BurnDatasetError> {
        let len = self.len();
        assert!(
            index < len,
            "Index out of bounds for DownloadedDataset: {index} >= {len}"
        );

        decode_item(
            &self.items[index],
            &self.name,
            self.version,
            index as u64,
            ItemLocation::Cached(&self.path),
        )
        .map_err(BurnDatasetError::new)
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

fn temporary_path(path: &Path) -> PathBuf {
    path.with_extension(format!("jsonl.{}.tmp", std::process::id()))
}

fn download_to(
    provider: &dyn DatasetProvider,
    name: &str,
    version: u32,
    path: &Path,
) -> Result<(), DatasetError> {
    let expected = provider.item_count(name, version)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = temporary_path(path);
    let actual = match write_download(provider, name, version, expected, &tmp_path) {
        Ok(actual) => actual,
        Err(error) => {
            remove_temporary_file(&tmp_path);
            return Err(error);
        }
    };

    if actual != expected {
        remove_temporary_file(&tmp_path);
        return Err(DatasetError::IncompleteDownload {
            name: name.to_string(),
            version,
            expected,
            actual,
        });
    }

    if let Err(error) = fs::rename(&tmp_path, path) {
        remove_temporary_file(&tmp_path);
        return Err(error.into());
    }

    Ok(())
}

fn write_download(
    provider: &dyn DatasetProvider,
    name: &str,
    version: u32,
    expected: u64,
    tmp_path: &Path,
) -> Result<u64, DatasetError> {
    let mut writer = BufWriter::new(File::create(tmp_path)?);
    let mut cursor = 0_u64;

    while cursor < expected {
        let page = provider.get_items(name, version, Some(cursor), Some(DOWNLOAD_PAGE_SIZE))?;
        if page.items.is_empty() {
            break;
        }

        for item in page.items {
            writer.write_all(&item)?;
            writer.write_all(b"\n")?;
            cursor += 1;
        }
    }

    writer.flush()?;
    Ok(cursor)
}

fn remove_temporary_file(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {}
    }
}

fn read_items(path: &Path) -> Result<Vec<Vec<u8>>, DatasetError> {
    let contents = fs::read(path)?;
    Ok(contents
        .split(|&byte| byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(<[u8]>::to_vec)
        .collect())
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use burn::data::dataset::Dataset;
    use serde::Deserialize;

    use super::{DOWNLOAD_PAGE_SIZE, DownloadedDataset, temporary_path};
    use crate::dataset::item::envelope_item;
    use crate::dataset::{DatasetError, DatasetItemsPage, DatasetProvider};

    #[derive(Debug, Clone, Deserialize, PartialEq)]
    struct TestAnnotation {
        value: u32,
    }

    struct FakeProvider<F, C> {
        stream: F,
        count: C,
    }

    impl<F, C> DatasetProvider for FakeProvider<F, C>
    where
        F: Fn(&str, u32, Option<u64>, Option<u32>) -> Result<DatasetItemsPage, DatasetError>
            + Send
            + Sync,
        C: Fn(&str, u32) -> Result<u64, DatasetError> + Send + Sync,
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

        fn item_count(&self, name: &str, version: u32) -> Result<u64, DatasetError> {
            (self.count)(name, version)
        }

        fn resolve_version(&self, _name: &str) -> Result<u32, DatasetError> {
            unreachable!("DownloadedDataset only receives resolved versions")
        }
    }

    fn item(value: u32) -> Vec<u8> {
        envelope_item(
            format!("example-{value}").as_bytes(),
            Some(serde_json::json!({ "value": value })),
        )
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
            stream: move |_name: &str, _version: u32, index: Option<u64>, limit: Option<u32>| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                let start = index.unwrap_or(0) as usize;
                let limit = limit.map(|value| value as usize).unwrap_or(total);
                let end = (start + limit).min(total);
                Ok(DatasetItemsPage {
                    items: (start..end).map(|index| item(index as u32)).collect(),
                })
            },
            count: move |_name: &str, _version: u32| Ok(total as u64),
        };
        let cache_root = tempfile::tempdir().unwrap();

        let dataset = DownloadedDataset::<TestAnnotation>::try_get_or_download(
            Arc::new(provider),
            "ds".to_string(),
            1,
            cache_root.path(),
        )
        .unwrap();

        assert_eq!(dataset.len(), total);
        assert_eq!(dataset.get(0).unwrap().example, b"example-0");
        assert_eq!(
            dataset.get(total - 1).unwrap().annotation,
            Some(TestAnnotation {
                value: (total - 1) as u32
            })
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(cache_file(cache_root.path(), "ds", 1).is_file());
    }

    #[test]
    fn given_server_clamps_pages_when_download_then_fetching_continues_to_item_count() {
        let total = 650_usize;
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let provider = FakeProvider {
            stream: move |_name: &str, _version: u32, index: Option<u64>, _limit: Option<u32>| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                let start = index.unwrap_or(0) as usize;
                let end = (start + 200).min(total);
                Ok(DatasetItemsPage {
                    items: (start..end).map(|index| item(index as u32)).collect(),
                })
            },
            count: move |_name: &str, _version: u32| Ok(total as u64),
        };
        let cache_root = tempfile::tempdir().unwrap();

        let dataset = DownloadedDataset::<TestAnnotation>::try_get_or_download(
            Arc::new(provider),
            "ds".to_string(),
            1,
            cache_root.path(),
        )
        .unwrap();

        assert_eq!(dataset.len(), total);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn given_cache_hit_when_download_then_provider_is_not_called() {
        let cache_root = tempfile::tempdir().unwrap();
        let path = cache_file(cache_root.path(), "ds", 1);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, [item(1), item(2)].join(&b'\n')).unwrap();

        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _index: Option<u64>, _limit: Option<u32>| {
                panic!("provider should not be called on a cache hit")
            },
            count: |_name: &str, _version: u32| {
                panic!("item count should not be called on a cache hit")
            },
        };

        let dataset = DownloadedDataset::<TestAnnotation>::try_get_or_download(
            Arc::new(provider),
            "ds".to_string(),
            1,
            cache_root.path(),
        )
        .unwrap();

        assert_eq!(dataset.len(), 2);
        assert_eq!(
            dataset.get(0).unwrap().annotation,
            Some(TestAnnotation { value: 1 })
        );
        assert_eq!(
            dataset.get(1).unwrap().annotation,
            Some(TestAnnotation { value: 2 })
        );
    }

    #[test]
    fn given_corrupted_cache_file_when_get_then_corrupt_cached_item_error_names_the_path() {
        let cache_root = tempfile::tempdir().unwrap();
        let path = cache_file(cache_root.path(), "ds", 1);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not json\n").unwrap();

        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _index: Option<u64>, _limit: Option<u32>| {
                panic!("provider should not be called on a cache hit")
            },
            count: |_name: &str, _version: u32| {
                panic!("item count should not be called on a cache hit")
            },
        };
        let dataset = DownloadedDataset::<TestAnnotation>::try_get_or_download(
            Arc::new(provider),
            "ds".to_string(),
            1,
            cache_root.path(),
        )
        .unwrap();

        let err = dataset.get(0).unwrap_err();
        let source = err
            .source()
            .expect("BurnDatasetError should carry DatasetError as its source");
        let dataset_err = source
            .downcast_ref::<DatasetError>()
            .expect("source should be DatasetError");

        assert!(matches!(
            dataset_err,
            DatasetError::CorruptCachedItem {
                index: 0,
                path: error_path,
                ..
            } if error_path == &path
        ));
    }

    #[test]
    fn given_provider_error_when_download_then_no_cache_or_temporary_file_is_left() {
        let provider = FakeProvider {
            stream: |name: &str, _version: u32, _index: Option<u64>, _limit: Option<u32>| {
                Err(DatasetError::DatasetNotFound {
                    name: name.to_string(),
                })
            },
            count: |_name: &str, _version: u32| Ok(1),
        };
        let cache_root = tempfile::tempdir().unwrap();
        let path = cache_file(cache_root.path(), "ds", 1);

        let result = DownloadedDataset::<TestAnnotation>::try_get_or_download(
            Arc::new(provider),
            "ds".to_string(),
            1,
            cache_root.path(),
        );

        assert!(result.is_err());
        assert!(!path.exists());
        assert!(!temporary_path(&path).exists());
    }

    #[test]
    fn given_empty_page_before_item_count_when_download_then_incomplete_error_removes_tmp_file() {
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, index: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: if index == Some(0) {
                        vec![item(1)]
                    } else {
                        vec![]
                    },
                })
            },
            count: |_name: &str, _version: u32| Ok(2),
        };
        let cache_root = tempfile::tempdir().unwrap();
        let path = cache_file(cache_root.path(), "ds", 4);

        let result = DownloadedDataset::<TestAnnotation>::try_get_or_download(
            Arc::new(provider),
            "ds".to_string(),
            4,
            cache_root.path(),
        );

        assert!(matches!(
            result,
            Err(DatasetError::IncompleteDownload {
                name,
                version: 4,
                expected: 2,
                actual: 1,
            }) if name == "ds"
        ));
        assert!(!path.exists());
        assert!(!temporary_path(&path).exists());
    }

    #[test]
    #[should_panic(expected = "Index out of bounds")]
    fn given_index_past_dataset_length_when_get_then_it_panics() {
        let cache_root = tempfile::tempdir().unwrap();
        let path = cache_file(cache_root.path(), "ds", 1);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, item(1)).unwrap();
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _index: Option<u64>, _limit: Option<u32>| unreachable!(),
            count: |_name: &str, _version: u32| unreachable!(),
        };
        let dataset = DownloadedDataset::<TestAnnotation>::try_get_or_download(
            Arc::new(provider),
            "ds".to_string(),
            1,
            cache_root.path(),
        )
        .unwrap();

        dataset.get(5).unwrap();
    }
}
