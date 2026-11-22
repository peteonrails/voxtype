//! A small OpenAI-compatible chat client, shared by meeting summaries, the
//! meeting assistant (Ask) and dictation cleanup (`[output.post_process]`).
//!
//! It speaks `POST {base_url}/chat/completions` with a bearer key and a
//! model id, streaming (server-sent events) or not. Two providers fill in
//! the endpoint:
//!
//! - [`Provider::OpenAi`]: any OpenAI-compatible server (llama-server,
//!   LM Studio, vLLM, Ollama's `/v1`), from a configured URL, model and key
//!   file.
//! - [`Provider::Omarchy`]: the text model Omarchy runs locally, found
//!   through Omarchy's documented interface (see [`omarchy`]).
//!
//! The key is never held in config or in memory longer than one request:
//! an [`Endpoint`] knows only where to read it, and it is read when a
//! request is made. It never reaches a log line or an error message.

pub mod omarchy;

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

/// Where an LLM request goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// The model Omarchy runs locally (`omarchy models`).
    Omarchy,
    /// Any OpenAI-compatible server, from configuration.
    OpenAi,
}

impl Provider {
    /// The config value: `omarchy` or `openai`.
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Omarchy => "omarchy",
            Provider::OpenAi => "openai",
        }
    }

    /// `omarchy` or `openai`; anything else is not an LLM provider.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "omarchy" => Some(Provider::Omarchy),
            "openai" => Some(Provider::OpenAi),
            _ => None,
        }
    }
}

/// What a consumer configured: the provider and, for `openai`, where the
/// server is. For `omarchy`, `model` optionally names the recipe to use.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmConfig {
    pub provider: Provider,
    /// Base URL including `/v1`, for `openai`.
    pub url: Option<String>,
    /// Model id (`openai`) or Omarchy recipe (`omarchy`).
    pub model: Option<String>,
    /// File holding the bearer key, for `openai`. Omarchy's key file is
    /// fixed by Omarchy.
    pub api_key_file: Option<PathBuf>,
    /// The `[...]` setting names, for error messages: the table that holds
    /// `url`, `model` and `api_key_file`, and their key names there.
    pub settings: SettingNames,
}

/// Names of the config keys an [`LlmConfig`] came from.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingNames {
    pub table: &'static str,
    pub url: &'static str,
    pub model: &'static str,
}

impl LlmConfig {
    /// Find the endpoint to talk to. For Omarchy this asks Omarchy which
    /// text model is running; it never starts one.
    pub fn resolve(&self, env: &omarchy::OmarchyEnv) -> Result<Endpoint, LlmError> {
        match self.provider {
            Provider::Omarchy => omarchy::discover(env, self.model.as_deref()),
            Provider::OpenAi => {
                let url = self
                    .url
                    .as_deref()
                    .map(str::trim)
                    .filter(|u| !u.is_empty())
                    .ok_or_else(|| {
                        LlmError::NotConfigured(format!(
                            "The openai backend needs a server URL. Set `{}` in [{}], \
                             for example \"http://127.0.0.1:8080/v1\".",
                            self.settings.url, self.settings.table
                        ))
                    })?;
                let model = self
                    .model
                    .as_deref()
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .ok_or_else(|| {
                        LlmError::NotConfigured(format!(
                            "The openai backend needs a model id. Set `{}` in [{}].",
                            self.settings.model, self.settings.table
                        ))
                    })?;
                Ok(Endpoint {
                    provider: Provider::OpenAi,
                    base_url: url.trim_end_matches('/').to_string(),
                    model: model.to_string(),
                    key_file: self.api_key_file.clone(),
                })
            }
        }
    }
}

/// A configured path with a leading `~/` expanded to the home directory.
pub fn expand_home(path: &str) -> PathBuf {
    let path = path.trim();
    match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

/// One resolved endpoint. Holds where the key is, never the key.
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    pub provider: Provider,
    /// Base URL including `/v1`, without a trailing slash.
    pub base_url: String,
    pub model: String,
    pub key_file: Option<PathBuf>,
}

impl Endpoint {
    /// How messages name it: "Omarchy (qwen3.6-35b at 127.0.0.1:11435)".
    pub fn label(&self) -> String {
        let who = match self.provider {
            Provider::Omarchy => "Omarchy",
            Provider::OpenAi => "OpenAI-compatible server",
        };
        format!("{who} ({} at {})", self.model, host_of(&self.base_url))
    }

    /// Short name for the UI: "Omarchy (qwen3.6-35b)", "Local (qwen3)".
    pub fn display_name(&self) -> String {
        match self.provider {
            Provider::Omarchy => format!("Omarchy ({})", self.model),
            Provider::OpenAi if self.is_local() => format!("Local ({})", self.model),
            Provider::OpenAi => format!("{} ({})", host_of(&self.base_url), self.model),
        }
    }

    /// `omarchy:<recipe>` or `openai:<model>`.
    pub fn id(&self) -> String {
        format!("{}:{}", self.provider.as_str(), self.model)
    }

    /// Whether text sent here stays on this machine. Omarchy's endpoints
    /// are loopback by construction (discovery refuses any other); an
    /// `openai` server counts only when its URL is a loopback address.
    pub fn is_local(&self) -> bool {
        self.provider == Provider::Omarchy || is_loopback_url(&self.base_url)
    }
}

/// `host:port` of a URL, for messages.
fn host_of(url: &str) -> &str {
    url.trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or(url)
}

/// Is `url`'s host a loopback address (127.0.0.0/8, ::1, localhost)?
pub fn is_loopback_url(url: &str) -> bool {
    let rest = match url.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") || scheme == "https" => rest,
        _ => return false,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // No credentials in the authority: "user@host" could hide the real host.
    if authority.contains('@') {
        return false;
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    let host = host.to_ascii_lowercase();
    host == "localhost"
        || host == "::1"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Why a request failed. Every message names the endpoint and, where there
/// is one, what to do. None of them contains the key.
#[derive(Debug, Error)]
pub enum LlmError {
    /// The provider is chosen but not usable as configured.
    #[error("{0}")]
    NotConfigured(String),

    /// Omarchy has no suitable text model running.
    #[error("{0}")]
    NotRunning(String),

    #[error("Could not read the API key from {}: {reason}", path.display())]
    Key { path: PathBuf, reason: String },

    #[error("{label} refused the API key (HTTP {status}).{hint}")]
    Unauthorized {
        label: String,
        status: u16,
        hint: String,
    },

    #[error("{label} did not answer within {secs}s.")]
    Timeout { label: String, secs: u64 },

    #[error("{label} could not be reached: {detail}")]
    Unreachable { label: String, detail: String },

    #[error("{label} returned an error (HTTP {status}): {detail}")]
    Http {
        label: String,
        status: u16,
        detail: String,
    },

    #[error("{label} sent a reply voxtype could not read: {detail}")]
    Malformed { label: String, detail: String },
}

impl LlmError {
    /// Whether the endpoint itself may have gone away or changed (the model
    /// was stopped or another one started), so finding it again may help.
    pub fn is_stale_endpoint(&self) -> bool {
        matches!(
            self,
            LlmError::Unreachable { .. }
                | LlmError::Unauthorized { .. }
                | LlmError::Http { status: 404, .. }
        )
    }
}

/// One chat request: a system prompt and a user message.
#[derive(Debug, Clone)]
pub struct ChatRequest<'a> {
    pub system: &'a str,
    pub user: &'a str,
    pub timeout: Duration,
    /// Stream the answer to `on_text` as it arrives.
    pub stream: bool,
}

/// Read a key file. The content never appears in the error.
pub fn read_key(path: &Path) -> Result<String, LlmError> {
    let text = std::fs::read_to_string(path).map_err(|e| LlmError::Key {
        path: path.to_path_buf(),
        reason: e.kind().to_string(),
    })?;
    let key = text.trim();
    if key.is_empty() {
        return Err(LlmError::Key {
            path: path.to_path_buf(),
            reason: "the file is empty".to_string(),
        });
    }
    Ok(key.to_string())
}

/// The model ids `endpoint` lists at `/models`, for `voxtype doctor`. No
/// model is run.
pub fn list_models(endpoint: &Endpoint, timeout: Duration) -> Result<Vec<String>, LlmError> {
    let key = endpoint.key_file.as_deref().map(read_key).transpose()?;
    let label = endpoint.label();
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2).min(timeout))
        .timeout(timeout)
        .build();
    let mut req = agent.get(&format!("{}/models", endpoint.base_url));
    if let Some(key) = key.as_deref() {
        req = req.set("Authorization", &format!("Bearer {key}"));
    }
    let response = req
        .call()
        .map_err(|e| describe_error(&label, endpoint, timeout, e))?;
    let v: Value = response.into_json().map_err(|e| LlmError::Malformed {
        label: label.clone(),
        detail: format!("the model list was not JSON ({e})"),
    })?;
    Ok(v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| m["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

/// Send `request` to `endpoint`. With `stream`, answer text goes to
/// `on_text` as it arrives; otherwise `on_text` gets the whole answer once.
/// Returns the answer with any `<think>` reasoning removed.
pub fn chat(
    endpoint: &Endpoint,
    request: &ChatRequest,
    on_text: &mut dyn FnMut(&str),
) -> Result<String, LlmError> {
    // Read at call time, so a rotated key is picked up and nothing caches it.
    let key = endpoint.key_file.as_deref().map(read_key).transpose()?;
    let mut body = json!({
        "model": endpoint.model,
        "messages": [
            {"role": "system", "content": request.system},
            {"role": "user", "content": request.user},
        ],
        "stream": request.stream,
    });
    // Omarchy's text recipes are thinking models (Qwen 3.x). Thinking would
    // spend the whole budget before the answer, so it is turned off where
    // the server's chat template takes the switch. Other servers may reject
    // an unknown field, so only Omarchy gets it.
    let thinking_off = endpoint.provider == Provider::Omarchy;
    if thinking_off {
        body["chat_template_kwargs"] = json!({"enable_thinking": false});
    }
    match send(endpoint, key.as_deref(), &body, request, on_text) {
        // A runtime that rejects the extra field: once more without it.
        Err(LlmError::Http { status: 400, .. }) if thinking_off => {
            tracing::debug!(
                "{} rejected chat_template_kwargs; retrying without it",
                endpoint.label()
            );
            if let Some(obj) = body.as_object_mut() {
                obj.remove("chat_template_kwargs");
            }
            send(endpoint, key.as_deref(), &body, request, on_text)
        }
        other => other,
    }
}

fn send(
    endpoint: &Endpoint,
    key: Option<&str>,
    body: &Value,
    request: &ChatRequest,
    on_text: &mut dyn FnMut(&str),
) -> Result<String, LlmError> {
    let label = endpoint.label();
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5).min(request.timeout))
        .timeout(request.timeout)
        .build();
    let url = format!("{}/chat/completions", endpoint.base_url);
    let mut req = agent.post(&url).set("Content-Type", "application/json");
    if let Some(key) = key {
        req = req.set("Authorization", &format!("Bearer {key}"));
    }
    tracing::debug!("Chat request to {} (stream: {})", label, request.stream);
    let response = req
        .send_string(&body.to_string())
        .map_err(|e| describe_error(&label, endpoint, request.timeout, e))?;

    let is_sse = response
        .header("Content-Type")
        .is_some_and(|t| t.to_ascii_lowercase().contains("text/event-stream"));
    if request.stream && is_sse {
        read_stream(&label, request.timeout, response.into_reader(), on_text)
    } else {
        let mut text = String::new();
        response
            .into_reader()
            .read_to_string(&mut text)
            .map_err(|e| read_error(&label, request.timeout, e))?;
        let answer = parse_completion(&label, &text)?;
        on_text(&answer);
        Ok(answer)
    }
}

/// An I/O error while reading the body: a timeout or a dropped connection.
fn read_error(label: &str, timeout: Duration, e: std::io::Error) -> LlmError {
    if matches!(
        e.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        LlmError::Timeout {
            label: label.to_string(),
            secs: timeout.as_secs().max(1),
        }
    } else {
        LlmError::Unreachable {
            label: label.to_string(),
            detail: format!("the connection dropped while reading the reply: {e}"),
        }
    }
}

fn describe_error(label: &str, endpoint: &Endpoint, timeout: Duration, e: ureq::Error) -> LlmError {
    match e {
        ureq::Error::Status(status @ (401 | 403), _) => LlmError::Unauthorized {
            label: label.to_string(),
            status,
            hint: match endpoint.provider {
                Provider::Omarchy => format!(
                    " Voxtype reads the key from {}; if Omarchy set the model up again, \
                     the key may have changed while the model kept the old one. \
                     Restart it from the local models panel.",
                    omarchy::KEY_FILE_DISPLAY
                ),
                Provider::OpenAi => {
                    " Check the key file set as `api_key_file` (or `openai_api_key_file`)."
                        .to_string()
                }
            },
        },
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            LlmError::Http {
                label: label.to_string(),
                status,
                detail: error_detail(&body).unwrap_or_else(|| format!("HTTP {status}")),
            }
        }
        ureq::Error::Transport(t) => {
            let text = t.to_string();
            if text.contains("timed out") || text.contains("timeout") {
                LlmError::Timeout {
                    label: label.to_string(),
                    secs: timeout.as_secs().max(1),
                }
            } else {
                LlmError::Unreachable {
                    label: label.to_string(),
                    detail: text,
                }
            }
        }
    }
}

/// The explanation in an error body: OpenAI's `{"error": {"message": ...}}`,
/// a plain `{"error": "..."}`, or the start of the text.
fn error_detail(body: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(body).ok();
    parsed
        .as_ref()
        .and_then(|v| {
            v["error"]["message"]
                .as_str()
                .or_else(|| v["error"].as_str())
                .or_else(|| v["message"].as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            let text = body.trim();
            (!text.is_empty()).then(|| text.chars().take(300).collect())
        })
}

/// The answer in a non-streamed completion.
fn parse_completion(label: &str, text: &str) -> Result<String, LlmError> {
    let malformed = |detail: String| LlmError::Malformed {
        label: label.to_string(),
        detail,
    };
    let v: Value = serde_json::from_str(text).map_err(|e| malformed(format!("not JSON ({e})")))?;
    if let Some(detail) = v
        .get("error")
        .filter(|e| !e.is_null())
        .and_then(|_| error_detail(text))
    {
        return Err(LlmError::Http {
            label: label.to_string(),
            status: 200,
            detail,
        });
    }
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| malformed("no choices[0].message.content in the reply".to_string()))?;
    Ok(strip_think(content))
}

/// Read a server-sent event stream of completion chunks.
fn read_stream(
    label: &str,
    timeout: Duration,
    reader: impl Read,
    on_text: &mut dyn FnMut(&str),
) -> Result<String, LlmError> {
    let mut raw = String::new();
    let mut filter = ThinkFilter::default();
    for line in BufReader::new(reader).lines() {
        let line = line.map_err(|e| read_error(label, timeout, e))?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        if data.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(data).map_err(|e| LlmError::Malformed {
            label: label.to_string(),
            detail: format!("a stream chunk was not JSON ({e})"),
        })?;
        if v.get("error").is_some_and(|e| !e.is_null()) {
            return Err(LlmError::Http {
                label: label.to_string(),
                status: 200,
                detail: error_detail(data).unwrap_or_else(|| "error in stream".to_string()),
            });
        }
        if let Some(text) = v["choices"][0]["delta"]["content"].as_str() {
            raw.push_str(text);
            let visible = filter.push(text);
            if !visible.is_empty() {
                on_text(&visible);
            }
        }
    }
    let tail = filter.finish();
    if !tail.is_empty() {
        on_text(&tail);
    }
    Ok(strip_think(&raw))
}

/// Remove `<think>...</think>` reasoning, including a reply that starts
/// inside it (a chat template that opens the block itself).
pub fn strip_think(text: &str) -> String {
    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";
    let mut out = text.to_string();
    while let Some(start) = out.find(OPEN) {
        match out[start..].find(CLOSE) {
            Some(rel) => out.replace_range(start..start + rel + CLOSE.len(), ""),
            None => out.truncate(start),
        }
    }
    if let Some(end) = out.find(CLOSE) {
        out = out[end + CLOSE.len()..].to_string();
    }
    out.trim().to_string()
}

/// Hides `<think>` blocks from streamed text as it arrives, holding back
/// only what could be the start of a tag.
#[derive(Debug, Default)]
struct ThinkFilter {
    inside: bool,
    pending: String,
}

impl ThinkFilter {
    fn push(&mut self, chunk: &str) -> String {
        self.pending.push_str(chunk);
        let mut out = String::new();
        loop {
            let tag = if self.inside { "</think>" } else { "<think>" };
            if let Some(i) = self.pending.find(tag) {
                if !self.inside {
                    out.push_str(&self.pending[..i]);
                }
                self.pending.drain(..i + tag.len());
                self.inside = !self.inside;
                continue;
            }
            // Keep the longest tail that could begin `tag`. Tags are ASCII,
            // so the cut is on a character boundary.
            let keep = (1..tag.len())
                .rev()
                .find(|&k| self.pending.ends_with(&tag[..k]))
                .unwrap_or(0);
            let cut = self.pending.len() - keep;
            if !self.inside {
                out.push_str(&self.pending[..cut]);
            }
            self.pending.drain(..cut);
            return out;
        }
    }

    fn finish(&mut self) -> String {
        if self.inside {
            String::new()
        } else {
            std::mem::take(&mut self.pending)
        }
    }
}

#[cfg(test)]
pub(crate) mod fake_server;

#[cfg(test)]
mod tests {
    use super::fake_server::{FakeServer, Reply};
    use super::*;

    fn endpoint(server: &FakeServer, provider: Provider, key_file: Option<PathBuf>) -> Endpoint {
        Endpoint {
            provider,
            base_url: server.base_url(),
            model: "qwen-test".into(),
            key_file,
        }
    }

    fn key_file(dir: &tempfile::TempDir) -> PathBuf {
        let path = dir.path().join("key");
        std::fs::write(&path, "test-key-123\n").unwrap();
        path
    }

    fn request(stream: bool) -> ChatRequest<'static> {
        ChatRequest {
            system: "sys",
            user: "hello",
            timeout: Duration::from_secs(5),
            stream,
        }
    }

    #[test]
    fn a_completion_returns_the_answer_and_sends_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let server = FakeServer::start(vec![Reply::completion("Hello there.")]);
        let ep = endpoint(&server, Provider::OpenAi, Some(key_file(&dir)));
        let mut seen = String::new();
        let answer = chat(&ep, &request(false), &mut |t| seen.push_str(t)).unwrap();
        assert_eq!(answer, "Hello there.");
        assert_eq!(seen, "Hello there.");
        let reqs = server.requests();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].method, "POST");
        assert_eq!(reqs[0].path, "/v1/chat/completions");
        assert_eq!(
            reqs[0].authorization.as_deref(),
            Some("Bearer test-key-123")
        );
        let body: Value = serde_json::from_str(&reqs[0].body).unwrap();
        assert_eq!(body["model"], "qwen-test");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "hello");
        // Only Omarchy gets the thinking switch.
        assert!(body.get("chat_template_kwargs").is_none());
    }

    #[test]
    fn omarchy_turns_thinking_off_and_retries_without_it_on_400() {
        let dir = tempfile::tempdir().unwrap();
        let server = FakeServer::start(vec![
            Reply::status(
                400,
                r#"{"error":{"message":"unknown field chat_template_kwargs"}}"#,
            ),
            Reply::completion("ok"),
        ]);
        let ep = endpoint(&server, Provider::Omarchy, Some(key_file(&dir)));
        assert_eq!(chat(&ep, &request(false), &mut |_| {}).unwrap(), "ok");
        let reqs = server.requests();
        let first: Value = serde_json::from_str(&reqs[0].body).unwrap();
        let second: Value = serde_json::from_str(&reqs[1].body).unwrap();
        assert_eq!(first["chat_template_kwargs"]["enable_thinking"], false);
        assert!(second.get("chat_template_kwargs").is_none());
    }

    #[test]
    fn a_401_is_an_unauthorized_error_without_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let server = FakeServer::start(vec![Reply::status(401, r#"{"error":"bad key"}"#)]);
        let ep = endpoint(&server, Provider::Omarchy, Some(key_file(&dir)));
        let err = chat(&ep, &request(false), &mut |_| {}).unwrap_err();
        assert!(
            matches!(err, LlmError::Unauthorized { status: 401, .. }),
            "{err}"
        );
        let msg = err.to_string();
        assert!(msg.contains("refused the API key"), "{msg}");
        assert!(msg.contains("Omarchy (qwen-test at 127.0.0.1:"), "{msg}");
        assert!(!msg.contains("test-key-123"), "{msg}");
        assert!(err.is_stale_endpoint());
    }

    #[test]
    fn a_slow_server_times_out() {
        let server = FakeServer::start(vec![Reply::completion("late").delayed(3_000)]);
        let ep = endpoint(&server, Provider::OpenAi, None);
        let req = ChatRequest {
            timeout: Duration::from_secs(1),
            ..request(false)
        };
        let err = chat(&ep, &req, &mut |_| {}).unwrap_err();
        assert!(matches!(err, LlmError::Timeout { secs: 1, .. }), "{err}");
    }

    #[test]
    fn nothing_listening_is_unreachable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        drop(listener);
        let ep = Endpoint {
            provider: Provider::OpenAi,
            base_url,
            model: "m".into(),
            key_file: None,
        };
        let err = chat(&ep, &request(false), &mut |_| {}).unwrap_err();
        assert!(matches!(err, LlmError::Unreachable { .. }), "{err}");
        assert!(err.is_stale_endpoint());
    }

    #[test]
    fn a_malformed_reply_is_reported() {
        for body in [
            "this is not json",
            r#"{"choices": []}"#,
            r#"{"choices": [{"message": {"role": "assistant"}}]}"#,
        ] {
            let server = FakeServer::start(vec![Reply::status(200, body)]);
            let ep = endpoint(&server, Provider::OpenAi, None);
            let err = chat(&ep, &request(false), &mut |_| {}).unwrap_err();
            assert!(matches!(err, LlmError::Malformed { .. }), "{body}: {err}");
        }
    }

    #[test]
    fn an_error_object_in_a_200_reply_is_an_error() {
        let server = FakeServer::start(vec![Reply::status(
            200,
            r#"{"error":{"message":"model is loading"}}"#,
        )]);
        let ep = endpoint(&server, Provider::OpenAi, None);
        let err = chat(&ep, &request(false), &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("model is loading"), "{err}");
    }

    #[test]
    fn a_missing_key_file_fails_before_any_request() {
        let server = FakeServer::start(vec![]);
        let ep = endpoint(
            &server,
            Provider::Omarchy,
            Some(PathBuf::from("/nonexistent/voxtype-test/key")),
        );
        let err = chat(&ep, &request(false), &mut |_| {}).unwrap_err();
        assert!(matches!(err, LlmError::Key { .. }), "{err}");
        assert!(server.requests().is_empty());
    }

    #[test]
    fn streams_deltas_and_hides_thinking() {
        let server = FakeServer::start(vec![Reply::stream(&[
            "<thi",
            "nk>plan the reply</th",
            "ink>Steve said ",
            "Friday [00:41].",
        ])]);
        let ep = endpoint(&server, Provider::OpenAi, None);
        let mut seen = Vec::new();
        let answer = chat(&ep, &request(true), &mut |t| seen.push(t.to_string())).unwrap();
        assert_eq!(answer, "Steve said Friday [00:41].");
        assert_eq!(seen.concat(), "Steve said Friday [00:41].");
        let body: Value = serde_json::from_str(&server.requests()[0].body).unwrap();
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn a_stream_request_answered_with_plain_json_still_works() {
        let server = FakeServer::start(vec![Reply::completion("whole")]);
        let ep = endpoint(&server, Provider::OpenAi, None);
        assert_eq!(chat(&ep, &request(true), &mut |_| {}).unwrap(), "whole");
    }

    #[test]
    fn strip_think_handles_blocks_and_an_open_start() {
        assert_eq!(strip_think("<think>a</think> b"), "b");
        assert_eq!(
            strip_think("x <think>a</think>y<think>c</think> z"),
            "x y z"
        );
        assert_eq!(strip_think("reasoning</think>\n\nanswer"), "answer");
        assert_eq!(strip_think("answer <think>never closed"), "answer");
        assert_eq!(strip_think("plain"), "plain");
    }

    #[test]
    fn loopback_urls() {
        for url in [
            "http://127.0.0.1:11435/v1",
            "http://localhost:8080/v1",
            "https://[::1]:8080/v1",
            "http://127.1.2.3/v1",
        ] {
            assert!(is_loopback_url(url), "{url}");
        }
        for url in [
            "https://api.openai.com/v1",
            "http://192.168.1.4:8080/v1",
            "http://127.0.0.1.evil.example/v1",
            "http://user@127.0.0.1@evil.example/v1",
            "127.0.0.1:8080",
            "ftp://127.0.0.1/",
        ] {
            assert!(!is_loopback_url(url), "{url}");
        }
    }

    #[test]
    fn openai_config_needs_a_url_and_a_model() {
        let names = SettingNames {
            table: "output.post_process",
            url: "url",
            model: "model",
        };
        let mut cfg = LlmConfig {
            provider: Provider::OpenAi,
            url: None,
            model: Some("m".into()),
            api_key_file: None,
            settings: names,
        };
        let env = omarchy::OmarchyEnv::disabled();
        let err = cfg.resolve(&env).unwrap_err();
        assert!(
            err.to_string().contains("`url` in [output.post_process]"),
            "{err}"
        );
        cfg.url = Some("http://127.0.0.1:8080/v1/".into());
        cfg.model = None;
        assert!(cfg
            .resolve(&env)
            .unwrap_err()
            .to_string()
            .contains("`model`"));
        cfg.model = Some("m".into());
        let ep = cfg.resolve(&env).unwrap();
        assert_eq!(ep.base_url, "http://127.0.0.1:8080/v1");
        assert!(ep.is_local());
        assert_eq!(ep.display_name(), "Local (m)");
    }
}
