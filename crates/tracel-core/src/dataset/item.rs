use std::error::Error;
use std::path::Path;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_with::base64::{Base64, Standard};
use serde_with::serde_as;

use super::DatasetError;

/// One item in a Tracel dataset.
///
/// The example payload is returned as raw bytes in [`DatasetItem::example`]; its format is
/// defined by the application that published the dataset. `A` is the application's annotation
/// type and must match the annotation schema stored with the dataset.
#[derive(Debug, Clone)]
pub struct DatasetItem<A = serde_json::Value> {
    /// The raw, base64-decoded example payload.
    pub example: Vec<u8>,
    /// The optional annotation associated with the example.
    pub annotation: Option<A>,
    /// The optional identifier of the source item from which this item was created.
    pub source_item_id: Option<String>,
    /// Optional application-defined metadata associated with the item.
    pub metadata: Option<serde_json::Value>,
}

#[serde_as]
#[derive(Deserialize)]
struct DatasetItemEnvelope {
    source_item_id: Option<String>,
    metadata: Option<serde_json::Value>,
    #[serde_as(as = "Base64<Standard>")]
    example_payload: Vec<u8>,
    #[allow(dead_code)]
    example_size_bytes: u64,
    annotation: Option<serde_json::Value>,
}

pub enum ItemLocation<'a> {
    Streamed,
    Cached(&'a Path),
}

pub fn decode_item<A>(
    raw: &[u8],
    name: &str,
    version: u32,
    index: u64,
    location: ItemLocation<'_>,
) -> Result<DatasetItem<A>, DatasetError>
where
    A: DeserializeOwned,
{
    let envelope = serde_json::from_slice::<DatasetItemEnvelope>(raw)
        .map_err(|source| corrupt_item_error(name, version, index, location, Box::new(source)))?;

    let annotation = envelope
        .annotation
        .map(serde_json::from_value)
        .transpose()
        .map_err(|source| DatasetError::AnnotationDecode {
            name: name.to_string(),
            version,
            index,
            source: Box::new(source),
        })?;

    Ok(DatasetItem {
        example: envelope.example_payload,
        annotation,
        source_item_id: envelope.source_item_id,
        metadata: envelope.metadata,
    })
}

fn corrupt_item_error(
    name: &str,
    version: u32,
    index: u64,
    location: ItemLocation<'_>,
    source: Box<dyn Error + Send + Sync>,
) -> DatasetError {
    match location {
        ItemLocation::Streamed => DatasetError::CorruptItem {
            name: name.to_string(),
            version,
            index,
            source,
        },
        ItemLocation::Cached(path) => DatasetError::CorruptCachedItem {
            name: name.to_string(),
            version,
            index,
            path: path.to_path_buf(),
            source,
        },
    }
}

#[cfg(test)]
pub fn envelope_item(example: &[u8], annotation: Option<serde_json::Value>) -> Vec<u8> {
    use serde::Serialize;

    #[serde_as]
    #[derive(Serialize)]
    struct Envelope<'a> {
        source_item_id: Option<&'a str>,
        metadata: Option<serde_json::Value>,
        #[serde_as(as = "Base64<Standard>")]
        example_payload: &'a [u8],
        example_size_bytes: u64,
        annotation: Option<serde_json::Value>,
        entry_idx: u64,
    }

    serde_json::to_vec(&Envelope {
        source_item_id: Some("source-1"),
        metadata: Some(serde_json::json!({ "split": "train" })),
        example_payload: example,
        example_size_bytes: example.len() as u64,
        annotation,
        entry_idx: 0,
    })
    .unwrap()
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::{DatasetItem, ItemLocation, decode_item, envelope_item};
    use crate::dataset::DatasetError;

    #[derive(Debug, Clone, Deserialize, PartialEq)]
    struct TestAnnotation {
        label: String,
    }

    #[test]
    fn given_valid_envelope_when_decode_then_all_item_fields_are_returned() {
        let raw = envelope_item(
            b"example bytes",
            Some(serde_json::json!({ "label": "cat" })),
        );

        let item: DatasetItem<TestAnnotation> =
            decode_item(&raw, "ds", 3, 7, ItemLocation::Streamed).unwrap();

        assert_eq!(item.example, b"example bytes");
        assert_eq!(
            item.annotation,
            Some(TestAnnotation {
                label: "cat".to_string()
            })
        );
        assert_eq!(item.source_item_id.as_deref(), Some("source-1"));
        assert_eq!(item.metadata, Some(serde_json::json!({ "split": "train" })));
    }

    #[test]
    fn given_null_annotation_when_decode_then_annotation_is_none() {
        let raw = envelope_item(b"example", None);

        let item: DatasetItem<TestAnnotation> =
            decode_item(&raw, "ds", 1, 0, ItemLocation::Streamed).unwrap();

        assert_eq!(item.annotation, None);
    }

    #[test]
    fn given_invalid_base64_when_decode_then_corrupt_item_is_returned() {
        let raw = br#"{
            "source_item_id": null,
            "metadata": null,
            "example_payload": "%%%",
            "example_size_bytes": 3,
            "annotation": null,
            "entry_idx": 0
        }"#;

        let result = decode_item::<TestAnnotation>(raw, "ds", 2, 4, ItemLocation::Streamed);

        assert!(matches!(
            result,
            Err(DatasetError::CorruptItem {
                name,
                version: 2,
                index: 4,
                ..
            }) if name == "ds"
        ));
    }

    #[test]
    fn given_mismatched_annotation_type_when_decode_then_annotation_decode_is_returned() {
        let raw = envelope_item(b"example", Some(serde_json::json!({ "score": 0.9 })));

        let result = decode_item::<TestAnnotation>(&raw, "ds", 5, 6, ItemLocation::Streamed);

        assert!(matches!(
            result,
            Err(DatasetError::AnnotationDecode {
                name,
                version: 5,
                index: 6,
                ..
            }) if name == "ds"
        ));
    }
}
