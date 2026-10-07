//! The inputs of a request: the JSON documents of its body, read as the body arrives.

use std::sync::mpsc;

use axum::body::Body;
use serde_json::Value;
use tokio_stream::StreamExt;

use crate::BoxError;

/// An input read from the body, or why it could not be read, which ends the inputs.
type Input = Result<Value, BoxError>;

/// Reads the JSON documents of `body` as it arrives and sends each to `inputs`, as [`Documents`]
/// reads them. A document that cannot be read, or a body that fails, is sent as the error that
/// ends the inputs.
pub async fn feed(body: Body, inputs: mpsc::Sender<Input>) {
    let mut documents = Documents::default();
    let mut body = body.into_data_stream();
    while let Some(chunk) = body.next().await {
        let read = match chunk {
            Ok(chunk) => documents.push(&chunk),
            Err(error) => vec![Err(
                format!("failed to read the request body: {error}").into()
            )],
        };
        if !send(&inputs, read) {
            return;
        }
    }
    send(&inputs, documents.end());
}

/// Sends `read` to `inputs`, and returns whether to read on: not after an error, nor once the job
/// takes no more inputs.
fn send(inputs: &mpsc::Sender<Input>, read: Vec<Input>) -> bool {
    for input in read {
        let failed = input.is_err();
        if inputs.send(input).is_err() || failed {
            return false;
        }
    }
    true
}

/// Splits a body into JSON documents as its bytes arrive: one per line (NDJSON), each read once
/// its line has arrived, or one document over several lines, such as a pretty-printed one, read
/// once the body has ended. Blank lines are skipped, and a body with no document holds one
/// `null`.
#[derive(Default)]
struct Documents {
    /// The bytes after the last line read.
    pending: Vec<u8>,
    /// Whether a document has been read.
    read_any: bool,
    /// Whether the body is one document over several lines.
    spans_lines: bool,
}

impl Documents {
    /// Takes `chunk`, the next bytes of the body, and returns the documents of the lines it ends,
    /// up to the first that cannot be read.
    fn push(&mut self, chunk: &[u8]) -> Vec<Input> {
        let mut read = Vec::new();
        let mut rest = chunk;
        while !self.spans_lines
            && let Some(end) = rest.iter().position(|&byte| byte == b'\n')
        {
            self.pending.extend_from_slice(&rest[..=end]);
            rest = &rest[end + 1..];
            if let Some(document) = self.read_line() {
                let failed = document.is_err();
                read.push(document);
                if failed {
                    return read;
                }
            }
        }
        self.pending.extend_from_slice(rest);
        read
    }

    /// The document left once the body has ended: its last line, or the document over several
    /// lines; `null` when the body held none.
    fn end(self) -> Vec<Input> {
        let rest = self.pending.trim_ascii();
        if !rest.is_empty() {
            vec![serde_json::from_slice(rest).map_err(invalid)]
        } else if self.read_any {
            Vec::new()
        } else {
            vec![Ok(Value::Null)]
        }
    }

    /// Reads the pending line: `None` when it is blank, or when it starts the body's first
    /// document and the document goes on past it.
    fn read_line(&mut self) -> Option<Input> {
        let line = self.pending.trim_ascii();
        if line.is_empty() {
            self.pending.clear();
            return None;
        }
        match serde_json::from_slice(line) {
            Ok(document) => {
                self.pending.clear();
                self.read_any = true;
                Some(Ok(document))
            }
            Err(error) if error.is_eof() && !self.read_any => {
                self.spans_lines = true;
                None
            }
            Err(error) => Some(Err(invalid(error))),
        }
    }
}

fn invalid(error: serde_json::Error) -> BoxError {
    format!("invalid JSON: {error}").into()
}

#[cfg(test)]
mod tests {
    use axum::body::Bytes;
    use serde_json::json;

    use super::*;

    fn shown(read: Vec<Input>) -> Vec<Result<Value, String>> {
        read.into_iter()
            .map(|input| input.map_err(|error| error.to_string()))
            .collect()
    }

    /// What `documents` reads from each of `chunks`, then from the end of the body.
    fn read(chunks: &[&str]) -> Vec<Vec<Result<Value, String>>> {
        let mut documents = Documents::default();
        let mut read: Vec<_> = chunks
            .iter()
            .map(|chunk| shown(documents.push(chunk.as_bytes())))
            .collect();
        read.push(shown(documents.end()));
        read
    }

    #[test]
    fn each_line_is_a_document_read_once_it_has_arrived() {
        assert_eq!(
            read(&["{\"text\": \"one\"}\n{\"te", "xt\": \"two\"}\n\n", "3"]),
            [
                vec![Ok(json!({"text": "one"}))],
                vec![Ok(json!({"text": "two"}))],
                vec![],
                vec![Ok(json!(3))],
            ]
        );
    }

    #[test]
    fn a_number_cut_off_by_a_chunk_goes_on_in_the_next() {
        assert_eq!(read(&["1", "2\n"]), [vec![], vec![Ok(json!(12))], vec![]]);
    }

    #[test]
    fn a_document_over_several_lines_is_read_once_the_body_has_ended() {
        assert_eq!(
            read(&["\n{\n  \"epochs\": 2,\n", "  \"lr\": 0.1\n}\n"]),
            [vec![], vec![], vec![Ok(json!({"epochs": 2, "lr": 0.1}))]]
        );
    }

    #[test]
    fn a_body_with_no_document_holds_null() {
        assert_eq!(read(&[]), [vec![Ok(Value::Null)]]);
        assert_eq!(read(&[" \n\n"]), [vec![], vec![Ok(Value::Null)]]);
    }

    #[test]
    fn reading_stops_at_a_line_that_is_not_a_document() {
        let not_json = read(&["1\nnope\n2\n"]);
        let cut_off = read(&["1\n{\"text\":\n"]);
        let unfinished = read(&["{\n\"text\":\n"]);

        for read in [&not_json[0], &cut_off[0]] {
            assert_eq!(read[0], Ok(json!(1)));
            assert!(
                matches!(&read[1], Err(error) if error.starts_with("invalid JSON: ")),
                "{read:?}"
            );
            assert_eq!(read.len(), 2);
        }
        assert!(
            matches!(&unfinished[1][..], [Err(error)] if error.starts_with("invalid JSON: ")),
            "{unfinished:?}"
        );
    }

    async fn fed(body: Body) -> Vec<Result<Value, String>> {
        let (inputs, received) = mpsc::channel();
        feed(body, inputs).await;
        shown(received.into_iter().collect())
    }

    #[tokio::test]
    async fn the_documents_of_the_body_are_fed_until_the_body_fails() {
        let chunks: Vec<Result<Bytes, std::io::Error>> = vec![
            Ok(Bytes::from("1\n2")),
            Ok(Bytes::from("\n")),
            Err(std::io::Error::other("connection reset")),
            Ok(Bytes::from("3\n")),
        ];

        let inputs = fed(Body::from_stream(tokio_stream::iter(chunks))).await;

        assert_eq!(
            inputs,
            [
                Ok(json!(1)),
                Ok(json!(2)),
                Err("failed to read the request body: connection reset".to_string())
            ]
        );
        assert_eq!(fed(Body::empty()).await, [Ok(Value::Null)]);
    }
}
