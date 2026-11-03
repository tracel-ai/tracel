use std::io::Read;

use reqwest::StatusCode;
use reqwest::blocking::Response;

use super::TransferError;

const ERROR_BODY_READ_LIMIT: u64 = 64 * 1024;

pub fn is_refusal(status: StatusCode) -> bool {
    matches!(status, StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN)
}

pub fn from_response(response: Response) -> TransferError {
    let status = response.status().as_u16();
    let mut body = String::new();
    let _ = response
        .take(ERROR_BODY_READ_LIMIT)
        .read_to_string(&mut body);

    TransferError::Refused {
        status,
        code: s3_error_code(&body),
    }
}

fn s3_error_code(body: &str) -> Option<String> {
    let start = body.find("<Code>")? + "<Code>".len();
    let length = body[start..].find("</Code>")?;
    let code = body[start..start + length].trim();

    (!code.is_empty()).then(|| code.to_string())
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Cursor, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    use crate::{FileTransferClient, ReqwestTransferClient, TransferError};

    fn answer_one_request(listener: TcpListener, status_line: &str, body: &str) {
        let (stream, _) = listener.accept().expect("accept the upload");
        let mut reader = BufReader::new(stream);
        let mut content_length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read a request header");
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                content_length = value.trim().parse().expect("a content length");
            }
        }
        let mut request_body = vec![0; content_length];
        std::io::Read::read_exact(&mut reader, &mut request_body).expect("read the part");

        let mut stream: TcpStream = reader.into_inner();
        write!(
            stream,
            "{status_line}\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("answer the upload");
    }

    #[test]
    fn a_part_the_store_refuses_names_its_error_code() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local store");
        let address = listener.local_addr().expect("local address");
        let store = thread::spawn(move || {
            answer_one_request(
                listener,
                "HTTP/1.1 400 Bad Request",
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error><Code>ExpiredToken</Code>\
                 <Message>The provided token has expired.</Message></Error>",
            )
        });
        let client = ReqwestTransferClient::with_client(
            reqwest::blocking::Client::builder()
                .no_proxy()
                .build()
                .expect("an HTTP client"),
        );

        let error = client
            .put_reader(
                &format!("http://{address}/weights.bin?partNumber=1"),
                Cursor::new(b"abcd".to_vec()),
                4,
            )
            .expect_err("the store refuses the part");
        store.join().expect("the store answered");

        assert!(
            matches!(
                &error,
                TransferError::Refused { status: 400, code: Some(code) } if code == "ExpiredToken"
            ),
            "{error:?}"
        );
    }
}
