//! A fake OpenAI-compatible server on a loopback port, for tests. It answers
//! queued replies in order, one per connection, and records each request.
//! Nothing here talks to a real model.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One canned answer.
#[derive(Debug, Clone)]
pub struct Reply {
    status: u16,
    content_type: &'static str,
    body: String,
    delay_ms: u64,
}

impl Reply {
    /// A non-streamed chat completion with `content` as the answer.
    pub fn completion(content: &str) -> Self {
        let body = serde_json::json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": content},
                         "finish_reason": "stop"}],
        })
        .to_string();
        Self {
            status: 200,
            content_type: "application/json",
            body,
            delay_ms: 0,
        }
    }

    /// A streamed completion sending each of `chunks` as a delta.
    pub fn stream(chunks: &[&str]) -> Self {
        let mut body = String::new();
        for chunk in chunks {
            let event = serde_json::json!({
                "choices": [{"index": 0, "delta": {"content": chunk}}]
            });
            body.push_str(&format!("data: {event}\n\n"));
        }
        body.push_str("data: [DONE]\n\n");
        Self {
            status: 200,
            content_type: "text/event-stream",
            body,
            delay_ms: 0,
        }
    }

    /// Any status with a raw body.
    pub fn status(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.to_string(),
            delay_ms: 0,
        }
    }

    /// `/v1/models` listing `ids`.
    pub fn models(ids: &[&str]) -> Self {
        let data: Vec<_> = ids
            .iter()
            .map(|id| serde_json::json!({"id": id, "object": "model"}))
            .collect();
        Self::status(
            200,
            &serde_json::json!({"object": "list", "data": data}).to_string(),
        )
    }

    /// Wait before answering.
    pub fn delayed(mut self, ms: u64) -> Self {
        self.delay_ms = ms;
        self
    }
}

/// What the server saw.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub authorization: Option<String>,
    pub body: String,
}

pub struct FakeServer {
    addr: std::net::SocketAddr,
    seen: Arc<Mutex<Vec<Request>>>,
}

impl FakeServer {
    /// Serve `replies` in order, then stop accepting.
    pub fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for reply in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                log.lock().unwrap().push(request);
                if reply.delay_ms > 0 {
                    std::thread::sleep(Duration::from_millis(reply.delay_ms));
                }
                let reason = match reply.status {
                    200 => "OK",
                    400 => "Bad Request",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    404 => "Not Found",
                    _ => "Error",
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.status,
                    reply.content_type,
                    reply.body.len(),
                    reply.body
                );
            }
        });
        Self { addr, seen }
    }

    /// `http://127.0.0.1:<port>/v1`
    pub fn base_url(&self) -> String {
        format!("http://{}/v1", self.addr)
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn requests(&self) -> Vec<Request> {
        self.seen.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<Request> {
    let mut buf = vec![0u8; 65536];
    let mut req = Vec::new();
    loop {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        req.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&req);
        if let Some(head_end) = text.find("\r\n\r\n") {
            let len = text[..head_end]
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if req.len() >= head_end + 4 + len {
                let head = &text[..head_end];
                let mut first = head.lines().next().unwrap_or("").split_whitespace();
                let method = first.next().unwrap_or("").to_string();
                let path = first.next().unwrap_or("").to_string();
                let authorization = head.lines().find_map(|l| {
                    let (name, value) = l.split_once(':')?;
                    name.eq_ignore_ascii_case("authorization")
                        .then(|| value.trim().to_string())
                });
                return Some(Request {
                    method,
                    path,
                    authorization,
                    body: text[head_end + 4..].to_string(),
                });
            }
        }
    }
}
