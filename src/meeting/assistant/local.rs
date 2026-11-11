//! The local fallback: Ollama, at `[meeting.summary] ollama_url` with
//! `ollama_model`, the same model the existing local summary backend uses.

use super::AssistantError;
use std::io::{BufRead, BufReader};
use std::time::Duration;

/// Generate with Ollama, streaming text to `on_text`. `json` asks Ollama to
/// constrain the output to JSON (summaries).
pub fn generate(
    url: &str,
    model: &str,
    prompt: &str,
    json: bool,
    timeout: Duration,
    on_text: &mut dyn FnMut(&str),
) -> Result<String, AssistantError> {
    let client = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(timeout)
        .build();
    let mut body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "system": super::runner::SYSTEM_PROMPT,
        "stream": true,
    });
    if json {
        body["format"] = serde_json::json!("json");
    }
    let endpoint = format!("{}/api/generate", url.trim_end_matches('/'));
    let response = client
        .post(&endpoint)
        .send_json(body)
        .map_err(|e| AssistantError::Local(format!("Ollama at {url} did not answer ({e})")))?;

    let mut answer = String::new();
    for line in BufReader::new(response.into_reader()).lines() {
        let line = line.map_err(|e| AssistantError::Local(format!("reading from Ollama: {e}")))?;
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(&line)
            .map_err(|e| AssistantError::Local(format!("unexpected Ollama reply: {e}")))?;
        if let Some(err) = v["error"].as_str() {
            return Err(AssistantError::Local(format!("Ollama: {err}")));
        }
        if let Some(text) = v["response"].as_str() {
            on_text(text);
            answer.push_str(text);
        }
        if v["done"].as_bool() == Some(true) {
            break;
        }
    }
    Ok(answer.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// A one-request HTTP server that streams `lines` back.
    fn serve(lines: &'static [&'static str]) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 65536];
            let mut req = Vec::new();
            loop {
                let n = s.read(&mut buf).unwrap();
                req.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&req);
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let len = text[..head_end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if req.len() >= head_end + 4 + len {
                        break;
                    }
                }
            }
            let body = lines.join("\n");
            write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            String::from_utf8_lossy(&req).into_owned()
        });
        (url, h)
    }

    #[test]
    fn streams_ollama_chunks() {
        let (url, h) = serve(&[
            r#"{"response":"Steve ","done":false}"#,
            r#"{"response":"said hi [00:41]","done":false}"#,
            r#"{"response":"","done":true}"#,
        ]);
        let mut seen = Vec::new();
        let answer = generate(
            &url,
            "llama3.2",
            "Q",
            false,
            Duration::from_secs(5),
            &mut |t| seen.push(t.to_string()),
        )
        .unwrap();
        assert_eq!(answer, "Steve said hi [00:41]");
        assert_eq!(seen, vec!["Steve ", "said hi [00:41]", ""]);
        let req = h.join().unwrap();
        assert!(req.contains("\"model\":\"llama3.2\""));
        assert!(!req.contains("\"format\""));
    }

    #[test]
    fn an_unreachable_ollama_is_a_local_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let err = generate(&url, "m", "Q", true, Duration::from_secs(2), &mut |_| {}).unwrap_err();
        assert!(matches!(err, AssistantError::Local(_)), "{err}");
    }
}
