//! The local fallback: Ollama, at `[meeting.summary] ollama_url` with
//! `ollama_model`, the same model the existing local summary backend uses.

use super::AssistantError;
use std::io::{BufRead, BufReader};
use std::time::Duration;

/// How an Ollama backend is named in messages: "Ollama (llama3:latest at
/// localhost:11434)".
pub fn ollama_label(url: &str, model: &str) -> String {
    let host = url
        .trim_end_matches('/')
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    format!("Ollama ({model} at {host})")
}

/// What went wrong talking to Ollama, in plain words. Ollama explains a
/// failed request in its JSON body (`{"error": "..."}`), which says far
/// more than the status code: a model that is not pulled, or a models
/// directory it cannot write.
pub fn describe_ollama_error(url: &str, model: &str, err: ureq::Error) -> String {
    let who = ollama_label(url, model);
    match err {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            let detail = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["error"].as_str().map(str::to_string))
                .or_else(|| {
                    let text = body.trim();
                    (!text.is_empty()).then(|| text.chars().take(300).collect())
                })
                .unwrap_or_else(|| format!("HTTP {code}"));
            format!("{who} returned an error: {detail}")
        }
        ureq::Error::Transport(t) => {
            format!("{who} did not answer: {t}. Is Ollama running (ollama serve)?")
        }
    }
}

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
        .map_err(|e| AssistantError::Local(describe_ollama_error(url, model, e)))?;

    let mut answer = String::new();
    for line in BufReader::new(response.into_reader()).lines() {
        let line = line.map_err(|e| {
            AssistantError::Local(format!(
                "{}: reading the reply failed: {e}",
                ollama_label(url, model)
            ))
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(&line).map_err(|e| {
            AssistantError::Local(format!(
                "{} sent an unexpected reply: {e}",
                ollama_label(url, model)
            ))
        })?;
        if let Some(err) = v["error"].as_str() {
            return Err(AssistantError::Local(format!(
                "{} returned an error: {err}",
                ollama_label(url, model)
            )));
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
        serve_status("200 OK", lines)
    }

    /// The same, answering with `status`.
    pub(crate) fn serve_status(
        status: &'static str,
        lines: &'static [&'static str],
    ) -> (String, std::thread::JoinHandle<String>) {
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
                "HTTP/1.1 {}\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status,
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
        assert!(err.to_string().contains("Ollama (m at 127.0.0.1:"), "{err}");
    }

    /// Pete's 1.3.0 test: Ollama answered 500 because it could not write
    /// its models directory. The message has to say that, not "500".
    #[test]
    fn an_ollama_error_body_is_in_the_message() {
        let (url, _h) = serve_status(
            "500 Internal Server Error",
            &[r#"{"error":"mkdir /opt/models/manifests-v2: permission denied"}"#],
        );
        let err = generate(
            &url,
            "llama3:latest",
            "Q",
            true,
            Duration::from_secs(5),
            &mut |_| {},
        )
        .unwrap_err();
        let host = url.trim_start_matches("http://");
        assert_eq!(
            err.to_string(),
            format!(
                "Ollama (llama3:latest at {host}) returned an error: mkdir /opt/models/manifests-v2: permission denied"
            )
        );
        assert_eq!(err.code(), "local_failed");
    }

    #[test]
    fn an_error_in_the_stream_names_ollama() {
        let (url, _h) = serve(&[r#"{"error":"model 'llama9' not found"}"#]);
        let err = generate(
            &url,
            "llama9",
            "Q",
            false,
            Duration::from_secs(5),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .ends_with("returned an error: model 'llama9' not found"),
            "{err}"
        );
    }
}
