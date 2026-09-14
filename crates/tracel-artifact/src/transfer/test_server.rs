//! A minimal HTTP/1.1 server for exercising the transfer clients over a real socket.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

pub struct TestServer {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<Received>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone)]
pub struct Received {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Received {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

impl TestServer {
    /// Serves `file` at `/file`, honouring `Range` requests when `ranges` is set, and accepts
    /// uploads at `/upload`. Every connection is closed after one exchange.
    pub fn serve(file: Vec<u8>, ranges: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let thread = {
            let received = Arc::clone(&received);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(stream) = stream {
                        handle(stream, &file, ranges, &received);
                    }
                }
            })
        };

        Self {
            addr,
            received,
            stop,
            thread: Some(thread),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    pub fn received(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle(mut stream: TcpStream, file: &[u8], ranges: bool, received: &Mutex<Vec<Received>>) {
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let chunked = request
        .header("transfer-encoding")
        .is_some_and(|value| value.eq_ignore_ascii_case("chunked"));
    let response = match (request.method.as_str(), request.path.as_str()) {
        _ if chunked => Response::status(501),
        ("GET", "/file") => match request.header("range").filter(|_| ranges) {
            Some(range) => partial(file, range),
            None => Response::body(200, file.to_vec(), None),
        },
        ("PUT", "/upload") => Response::status(200),
        _ => Response::status(404),
    };
    received.lock().unwrap().push(request);

    let _ = stream.write_all(&response.encode());
    let _ = stream.shutdown(std::net::Shutdown::Write);
}

fn read_request(stream: &mut TcpStream) -> Option<Received> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(end) = find(&buffer, b"\r\n\r\n") {
            break end;
        }
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    };

    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_string();
    let path = request_line.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect();

    let content_length = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = head_end + 4;
    while buffer.len() - body_start < content_length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    Some(Received {
        method,
        path,
        headers,
        body: buffer[body_start..body_start + content_length].to_vec(),
    })
}

fn partial(file: &[u8], range: &str) -> Response {
    let Some((start, end)) = range
        .strip_prefix("bytes=")
        .and_then(|spec| spec.split_once('-'))
        .and_then(|(start, end)| Some((start.parse::<usize>().ok()?, end.parse::<usize>().ok()?)))
    else {
        return Response::status(400);
    };
    let end = end.min(file.len() - 1);
    Response::body(
        206,
        file[start..=end].to_vec(),
        Some(format!("bytes {start}-{end}/{}", file.len())),
    )
}

struct Response {
    status: u16,
    body: Vec<u8>,
    content_range: Option<String>,
}

impl Response {
    fn status(status: u16) -> Self {
        Self::body(status, Vec::new(), None)
    }

    fn body(status: u16, body: Vec<u8>, content_range: Option<String>) -> Self {
        Self {
            status,
            body,
            content_range,
        }
    }

    fn encode(self) -> Vec<u8> {
        let reason = match self.status {
            200 => "OK",
            206 => "Partial Content",
            400 => "Bad Request",
            404 => "Not Found",
            _ => "Not Implemented",
        };
        let mut encoded = format!(
            "HTTP/1.1 {} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.status,
            self.body.len()
        );
        if let Some(content_range) = self.content_range {
            encoded.push_str(&format!("Content-Range: {content_range}\r\n"));
        }
        encoded.push_str("\r\n");
        let mut bytes = encoded.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
