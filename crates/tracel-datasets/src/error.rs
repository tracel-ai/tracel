use std::error::Error;

use crate::VersionSpec;

/// Errors surfaced by the dataset capability.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DatasetsError {
    /// No dataset by this name is in scope.
    #[error("dataset '{name}' not found")]
    DatasetNotFound {
        /// Name that was looked up.
        name: String,
    },

    /// The dataset exists, but not the version asked for.
    #[error("dataset '{dataset}' has no {version}")]
    VersionNotFound {
        /// Dataset that was looked up.
        dataset: String,
        /// Version that was asked for.
        version: VersionSpec,
    },

    /// An item envelope could not be read.
    #[error("item {index} of dataset '{dataset}' version {version} is unreadable: {problem}")]
    Item {
        /// Dataset the item belongs to.
        dataset: String,
        /// Version the item belongs to.
        version: u32,
        /// Position of the item in the version.
        index: u64,
        /// What made it unreadable.
        problem: String,
    },

    /// An annotation did not match the requested type.
    #[error(
        "annotation of item {index} in dataset '{dataset}' version {version} \
         does not match the requested type: {problem}"
    )]
    Annotation {
        /// Dataset the item belongs to.
        dataset: String,
        /// Version the item belongs to.
        version: u32,
        /// Position of the item in the version.
        index: u64,
        /// How the annotation differed.
        problem: String,
    },

    /// Fewer items arrived than were asked for.
    #[error("dataset '{dataset}' version {version} yielded {actual} of {expected} items")]
    Incomplete {
        /// Dataset that was read.
        dataset: String,
        /// Version that was read.
        version: u32,
        /// Item count the version reports.
        expected: u64,
        /// Item count actually received.
        actual: u64,
    },

    /// Two items in one publication claimed the same source identity.
    #[error("source item id '{source_item_id}' was offered more than once")]
    DuplicateItem {
        /// The identifier offered twice.
        source_item_id: String,
    },

    /// The backend's own error, kept whole.
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

impl DatasetsError {
    /// Wraps a backend's own error without interpreting it.
    pub fn other(error: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self::Other(error.into())
    }

    /// Whether a dataset or a version of one was missing.
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Self::DatasetNotFound { .. } | Self::VersionNotFound { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_version_names_what_was_asked_for() {
        let by_number = DatasetsError::VersionNotFound {
            dataset: "mnist".to_string(),
            version: VersionSpec::Fixed(7),
        };
        let latest = DatasetsError::VersionNotFound {
            dataset: "mnist".to_string(),
            version: VersionSpec::Latest,
        };

        assert_eq!(by_number.to_string(), "dataset 'mnist' has no version 7");
        assert_eq!(latest.to_string(), "dataset 'mnist' has no latest version");
    }
}
