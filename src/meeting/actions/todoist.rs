//! Todoist: action items as tasks.
//!
//! Everything that adds a task goes through [`TodoistClient`], so the rest
//! of voxtype does not care how the account was connected, and tests use
//! a fake. The access token lives in `$XDG_STATE_HOME/voxtype/todoist.json`
//! (mode 600), never in config.toml.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Todoist's API root.
pub const API_BASE: &str = "https://api.todoist.com";

/// A task to add.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct NewTask {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// YYYY-MM-DD
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    /// What Todoist parses itself, used for a day with a time of day
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

/// Why a task was not added, each saying what to do.
#[derive(Debug, thiserror::Error)]
pub enum TodoistError {
    #[error("Todoist is not connected. Connect it with:\n  voxtype meeting todoist connect")]
    NotConnected,
    #[error(
        "Todoist refused the connection (it may have been revoked). Connect again with:\n  voxtype meeting todoist connect"
    )]
    Unauthorized,
    #[error("Todoist answered {status}: {message}")]
    Http { status: u16, message: String },
    #[error("Could not reach Todoist: {0}")]
    Network(String),
    #[error("Could not read or write {path}: {message}")]
    Store { path: String, message: String },
}

/// Adds tasks to a Todoist account.
pub trait TodoistClient {
    /// Add `task`, returning its Todoist ID.
    fn add_task(&self, task: &NewTask) -> Result<String, TodoistError>;
}

/// What `todoist.json` holds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tokens {
    /// Empty while voxtype is registered with Todoist but not connected
    #[serde(default)]
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix seconds the access token stops working, when it expires
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// The OAuth client the tokens were issued to, needed to refresh them
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// The loopback addresses `client_id` was registered with
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirect_uris: Vec<String>,
}

impl Tokens {
    pub fn connected(&self) -> bool {
        !self.access_token.trim().is_empty()
    }
}

/// `todoist.json`, readable by the user alone.
#[derive(Debug, Clone)]
pub struct TokenStore {
    pub path: PathBuf,
}

impl TokenStore {
    /// The store under voxtype's state directory.
    pub fn default_path() -> PathBuf {
        crate::config::Config::state_dir().join("todoist.json")
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn error(&self, e: impl std::fmt::Display) -> TodoistError {
        TodoistError::Store {
            path: self.path.display().to_string(),
            message: e.to_string(),
        }
    }

    /// The saved tokens, or `None` when Todoist was never connected.
    pub fn load(&self) -> Result<Option<Tokens>, TodoistError> {
        match std::fs::read_to_string(&self.path) {
            Ok(json) => serde_json::from_str(&json)
                .map(Some)
                .map_err(|e| self.error(e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(self.error(e)),
        }
    }

    /// Save `tokens`, created mode 600 and swapped in whole, so a reader
    /// never sees a partial file and no one else ever sees the token.
    pub fn save(&self, tokens: &Tokens) -> Result<(), TodoistError> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let dir = self.path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).map_err(|e| self.error(e))?;
        let staged = dir.join(format!(".todoist.json.{}", std::process::id()));
        let _ = std::fs::remove_file(&staged);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&staged)
            .map_err(|e| self.error(e))?;
        let json = serde_json::to_string_pretty(tokens).map_err(|e| self.error(e))?;
        file.write_all(json.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| self.error(e))?;
        std::fs::rename(&staged, &self.path).map_err(|e| self.error(e))
    }

    /// Forget the connection. Returns whether there was one.
    pub fn clear(&self) -> Result<bool, TodoistError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(self.error(e)),
        }
    }
}

/// Todoist's REST API, with the token from a [`TokenStore`], or from a
/// file holding a personal API token (`[meeting.actions]
/// todoist_token_file`), for a machine with no browser.
pub struct HttpTodoist {
    pub base: String,
    pub store: TokenStore,
    pub token_file: Option<PathBuf>,
    agent: ureq::Agent,
}

/// The personal API token in `path`, trimmed.
pub fn read_token_file(path: &Path) -> Result<String, TodoistError> {
    let token = std::fs::read_to_string(path)
        .map_err(|e| TodoistError::Store {
            path: path.display().to_string(),
            message: e.to_string(),
        })?
        .trim()
        .to_string();
    if token.is_empty() {
        return Err(TodoistError::Store {
            path: path.display().to_string(),
            message: "the file is empty".into(),
        });
    }
    Ok(token)
}

impl HttpTodoist {
    pub fn new(base: impl Into<String>, store: TokenStore) -> Self {
        Self {
            base: base.into(),
            store,
            token_file: None,
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(20))
                .build(),
        }
    }

    /// Use the personal API token in `path` instead of the stored tokens.
    pub fn with_token_file(mut self, path: Option<PathBuf>) -> Self {
        self.token_file = path;
        self
    }

    /// The access token, refreshed first when it has expired and can be.
    fn token(&self) -> Result<String, TodoistError> {
        if let Some(path) = &self.token_file {
            return read_token_file(path);
        }
        let tokens = self
            .store
            .load()?
            .filter(Tokens::connected)
            .ok_or(TodoistError::NotConnected)?;
        let expired = tokens
            .expires_at
            .is_some_and(|at| at <= chrono::Utc::now().timestamp() + 60);
        if !expired {
            return Ok(tokens.access_token);
        }
        match (&tokens.refresh_token, &tokens.client_id) {
            (Some(refresh), Some(client_id)) => self.refresh(&tokens, refresh, client_id),
            _ => Err(TodoistError::Unauthorized),
        }
    }

    /// Trade the refresh token for new tokens (a public client: no secret).
    /// Todoist rotates the refresh token, so the new one is saved at once.
    fn refresh(
        &self,
        old: &Tokens,
        refresh: &str,
        client_id: &str,
    ) -> Result<String, TodoistError> {
        #[derive(Deserialize)]
        struct Reply {
            access_token: String,
            refresh_token: Option<String>,
            expires_in: Option<i64>,
            scope: Option<String>,
        }
        let reply: Reply = self
            .agent
            .post(&format!("{}/oauth/access_token", self.base))
            .send_form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh),
                ("client_id", client_id),
            ])
            .map_err(http_error)?
            .into_json()
            .map_err(|e| TodoistError::Network(e.to_string()))?;
        let tokens = Tokens {
            access_token: reply.access_token.clone(),
            refresh_token: reply.refresh_token.or_else(|| old.refresh_token.clone()),
            expires_at: reply.expires_in.map(|s| chrono::Utc::now().timestamp() + s),
            client_id: old.client_id.clone(),
            scope: reply.scope.or_else(|| old.scope.clone()),
            redirect_uris: old.redirect_uris.clone(),
        };
        self.store.save(&tokens)?;
        Ok(reply.access_token)
    }
}

fn http_error(e: ureq::Error) -> TodoistError {
    match e {
        ureq::Error::Status(401 | 403, _) => TodoistError::Unauthorized,
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .or_else(|| v.get("error_description"))
                        .and_then(|m| m.as_str().map(str::to_string))
                })
                .unwrap_or_else(|| body.chars().take(200).collect());
            TodoistError::Http { status, message }
        }
        ureq::Error::Transport(t) => TodoistError::Network(t.to_string()),
    }
}

impl TodoistClient for HttpTodoist {
    fn add_task(&self, task: &NewTask) -> Result<String, TodoistError> {
        let token = self.token()?;
        let reply: serde_json::Value = self
            .agent
            .post(&format!("{}/api/v1/tasks", self.base))
            .set("Authorization", &format!("Bearer {token}"))
            .send_json(task)
            .map_err(http_error)?
            .into_json()
            .map_err(|e| TodoistError::Network(e.to_string()))?;
        reply
            .get("id")
            .and_then(|id| {
                id.as_str()
                    .map(str::to_string)
                    .or_else(|| id.as_u64().map(|n| n.to_string()))
            })
            .ok_or_else(|| TodoistError::Http {
                status: 200,
                message: "no task ID in the reply".into(),
            })
    }
}

/// A [`TodoistClient`] that records tasks instead of sending them.
#[cfg(test)]
#[derive(Default)]
pub struct FakeTodoist {
    pub added: std::cell::RefCell<Vec<NewTask>>,
    /// Fail every task whose content contains this
    pub fail_on: Option<String>,
}

#[cfg(test)]
impl TodoistClient for FakeTodoist {
    fn add_task(&self, task: &NewTask) -> Result<String, TodoistError> {
        if self
            .fail_on
            .as_deref()
            .is_some_and(|f| task.content.contains(f))
        {
            return Err(TodoistError::Http {
                status: 400,
                message: "bad task".into(),
            });
        }
        let mut added = self.added.borrow_mut();
        added.push(task.clone());
        Ok(format!("task-{}", added.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn tokens_are_saved_readable_by_the_user_alone() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::new(dir.path().join("state/voxtype/todoist.json"));
        assert_eq!(store.load().unwrap(), None);
        let tokens = Tokens {
            access_token: "secret".into(),
            refresh_token: Some("r1".into()),
            expires_at: Some(42),
            client_id: Some("tdd_x".into()),
            scope: None,
            redirect_uris: vec![],
        };
        store.save(&tokens).unwrap();
        let mode = std::fs::metadata(&store.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(store.load().unwrap(), Some(tokens));
        assert!(store.clear().unwrap());
        assert!(!store.clear().unwrap());
        assert_eq!(store.load().unwrap(), None);
    }

    /// A one-request HTTP server: answers `status` with `body` and hands
    /// back what it was sent.
    fn serve_once(status: u16, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut buf = vec![0; length];
            reader.read_exact(&mut buf).unwrap();
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            head + &String::from_utf8(buf).unwrap()
        });
        (base, handle)
    }

    #[test]
    fn a_task_is_posted_with_the_bearer_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::new(dir.path().join("todoist.json"));
        store
            .save(&Tokens {
                access_token: "tok".into(),
                ..Default::default()
            })
            .unwrap();
        let (base, server) = serve_once(200, r#"{"id": "6XGg", "content": "Tag"}"#);
        let client = HttpTodoist::new(base, store);
        let id = client
            .add_task(&NewTask {
                content: "Tag the release".into(),
                due_date: Some("2026-11-13".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(id, "6XGg");
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /api/v1/tasks "), "{request}");
        assert!(request.contains("Authorization: Bearer tok"), "{request}");
        let body: serde_json::Value =
            serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"content": "Tag the release", "due_date": "2026-11-13"})
        );
    }

    #[test]
    fn errors_say_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::new(dir.path().join("todoist.json"));
        let client = HttpTodoist::new("http://127.0.0.1:9", store.clone());
        let err = client.add_task(&NewTask::default()).unwrap_err();
        assert!(matches!(err, TodoistError::NotConnected));
        assert!(err.to_string().contains("voxtype meeting todoist connect"));

        store
            .save(&Tokens {
                access_token: "revoked".into(),
                ..Default::default()
            })
            .unwrap();
        let (base, server) = serve_once(401, r#"{"error": "unauthorized"}"#);
        let client = HttpTodoist::new(base, store.clone());
        assert!(matches!(
            client.add_task(&NewTask::default()),
            Err(TodoistError::Unauthorized)
        ));
        server.join().unwrap();

        let (base, server) = serve_once(400, r#"{"error": "Content is required"}"#);
        let client = HttpTodoist::new(base, store.clone());
        let err = client.add_task(&NewTask::default()).unwrap_err();
        assert_eq!(err.to_string(), "Todoist answered 400: Content is required");
        server.join().unwrap();

        // Expired with no way to refresh: connect again.
        store
            .save(&Tokens {
                access_token: "old".into(),
                expires_at: Some(1),
                ..Default::default()
            })
            .unwrap();
        let client = HttpTodoist::new("http://127.0.0.1:9", store);
        assert!(matches!(
            client.add_task(&NewTask::default()),
            Err(TodoistError::Unauthorized)
        ));
    }

    #[test]
    fn a_personal_token_file_is_used_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("todoist-token");
        std::fs::write(&file, "  personal123\n").unwrap();
        let (base, server) = serve_once(200, r#"{"id": "1"}"#);
        // No stored tokens at all: the file is enough.
        let client = HttpTodoist::new(base, TokenStore::new(dir.path().join("todoist.json")))
            .with_token_file(Some(file.clone()));
        client
            .add_task(&NewTask {
                content: "x".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(server
            .join()
            .unwrap()
            .contains("Authorization: Bearer personal123"));
        std::fs::write(&file, "\n").unwrap();
        let err = read_token_file(&file).unwrap_err();
        assert!(err.to_string().contains("empty"), "{err}");
    }

    #[test]
    fn an_expired_token_is_refreshed_and_the_rotated_one_saved() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::new(dir.path().join("todoist.json"));
        store
            .save(&Tokens {
                access_token: "old".into(),
                refresh_token: Some("r1".into()),
                expires_at: Some(1),
                client_id: Some("tdd_client".into()),
                scope: Some("task:add".into()),
                redirect_uris: vec![],
            })
            .unwrap();
        let (base, server) = serve_once(
            200,
            r#"{"access_token": "new", "refresh_token": "r2", "expires_in": 3600, "token_type": "Bearer"}"#,
        );
        let client = HttpTodoist::new(base, store.clone());
        assert_eq!(client.token().unwrap(), "new");
        let request = server.join().unwrap();
        assert!(
            request.starts_with("POST /oauth/access_token "),
            "{request}"
        );
        let form = request.split("\r\n\r\n").nth(1).unwrap();
        assert!(form.contains("grant_type=refresh_token"));
        assert!(form.contains("refresh_token=r1"));
        assert!(form.contains("client_id=tdd_client"));
        assert!(!form.contains("client_secret"), "a public client has none");
        let saved = store.load().unwrap().unwrap();
        assert_eq!(saved.access_token, "new");
        assert_eq!(saved.refresh_token.as_deref(), Some("r2"));
        assert!(saved.expires_at.unwrap() > chrono::Utc::now().timestamp());
        assert_eq!(saved.scope.as_deref(), Some("task:add"));
    }
}
