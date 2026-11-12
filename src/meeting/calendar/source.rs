//! The calendar's private iCal link: where it is kept, fetching it, and
//! the events cached from it.
//!
//! The link grants read access to the whole calendar, so it is treated as
//! a secret: kept in `$XDG_STATE_HOME/voxtype/calendar.json` (mode 600),
//! never in config.toml, never logged, and redacted to its host in every
//! message. The feed and the events parsed from it are cached beside it,
//! also mode 600, since event titles are private too.

use super::ics::{self, Event};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The link and what is known about fetching it.
pub const SETTINGS_FILE: &str = "calendar.json";
/// The feed as last fetched, for a conditional fetch to fall back on.
pub const FEED_FILE: &str = "calendar.ics";
/// Events from shortly before the last fetch to 48 hours after it.
pub const EVENTS_FILE: &str = "calendar-events.json";

pub use crate::cli::CALENDAR_LINK_HELP as LINK_HELP;

/// How often the daemon fetches the feed.
pub const REFRESH_EVERY: Duration = Duration::minutes(15);
/// How far ahead events are cached.
pub const AHEAD: Duration = Duration::hours(48);
/// Largest feed read; a calendar with years of history fits easily.
const MAX_FEED_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IcsSettings {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    /// Unix seconds of the last successful fetch
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<i64>,
    /// The last failure, already redacted
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Events cached from the feed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Cached {
    pub fetched_at: i64,
    pub events: Vec<Event>,
}

/// Why the feed could not be used. No message carries the link.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum IcsError {
    #[error("No calendar link is set. Set one with:\n  voxtype meeting calendar set-ics <link>")]
    NotSet,
    #[error("That is not a calendar link: it must start with https://, http:// or webcal://")]
    BadLink,
    #[error("The calendar server ({host}) answered {status}{hint}")]
    Http {
        host: String,
        status: u16,
        hint: &'static str,
    },
    #[error("Could not reach the calendar server ({host}): {kind}")]
    Network { host: String, kind: String },
    #[error("The calendar link ({host}) did not return a calendar: {detail}")]
    NotCalendar { host: String, detail: String },
    #[error("Could not read or write the calendar files: {0}")]
    Io(String),
}

/// `webcal://` (what calendar apps hand out) is the same feed over https.
pub fn normalize_link(raw: &str) -> Result<String, IcsError> {
    let raw = raw.trim();
    let link = if let Some(rest) = raw
        .strip_prefix("webcal://")
        .or_else(|| raw.strip_prefix("webcals://"))
    {
        format!("https://{rest}")
    } else {
        raw.to_string()
    };
    let rest = link
        .strip_prefix("https://")
        .or_else(|| link.strip_prefix("http://"))
        .ok_or(IcsError::BadLink)?;
    if rest.split(['/', '?', '#']).next().unwrap_or("").is_empty()
        || link.contains(char::is_whitespace)
    {
        return Err(IcsError::BadLink);
    }
    Ok(link)
}

/// The link's host alone, which is all that is ever shown of it.
pub fn host(link: &str) -> String {
    link.split("://")
        .nth(1)
        .unwrap_or("")
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("")
        .to_string()
}

/// `message` with the link, and its path and query, replaced, for text
/// that came from somewhere voxtype does not control.
pub fn scrub(message: &str, link: &str) -> String {
    let mut out = message.to_string();
    let without_scheme = link.split("://").nth(1).unwrap_or(link);
    let path = without_scheme
        .find('/')
        .map(|i| &without_scheme[i..])
        .unwrap_or("");
    for secret in [link, without_scheme, path] {
        if secret.len() > 1 {
            out = out.replace(secret, "<calendar link>");
        }
    }
    out
}

/// Where the calendar files live.
#[derive(Debug, Clone)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    /// The store under voxtype's state directory.
    pub fn default_dir() -> PathBuf {
        crate::config::Config::state_dir()
    }

    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn settings(&self) -> Option<IcsSettings> {
        let json = std::fs::read_to_string(self.path(SETTINGS_FILE)).ok()?;
        serde_json::from_str::<IcsSettings>(&json)
            .ok()
            .filter(|s| !s.url.trim().is_empty())
    }

    pub fn save_settings(&self, settings: &IcsSettings) -> Result<(), IcsError> {
        let json = serde_json::to_vec_pretty(settings).map_err(|e| IcsError::Io(e.to_string()))?;
        write_private(&self.path(SETTINGS_FILE), &json)
    }

    /// Set a new link, forgetting everything fetched from the old one.
    pub fn set_link(&self, link: &str) -> Result<(), IcsError> {
        let url = normalize_link(link)?;
        self.forget_feed();
        self.save_settings(&IcsSettings {
            url,
            ..Default::default()
        })
    }

    /// Remove the link and everything cached from it. Returns whether a
    /// link was set.
    pub fn clear(&self) -> bool {
        let had = self.settings().is_some();
        let _ = std::fs::remove_file(self.path(SETTINGS_FILE));
        self.forget_feed();
        had
    }

    fn forget_feed(&self) {
        let _ = std::fs::remove_file(self.path(FEED_FILE));
        let _ = std::fs::remove_file(self.path(EVENTS_FILE));
    }

    pub fn cached(&self) -> Option<Cached> {
        serde_json::from_str(&std::fs::read_to_string(self.path(EVENTS_FILE)).ok()?).ok()
    }

    /// Whether the cache is missing or older than [`REFRESH_EVERY`].
    pub fn stale(&self, now: DateTime<Utc>) -> bool {
        self.settings()
            .and_then(|s| s.fetched_at)
            .is_none_or(|at| now.timestamp() - at >= REFRESH_EVERY.num_seconds())
    }

    /// Fetch the feed (conditionally, when the server gave an ETag or a
    /// Last-Modified before), parse it, and cache the events from a day
    /// before `now` to [`AHEAD`] after. A failure is recorded, redacted,
    /// for `meeting calendar status`.
    pub fn refresh(
        &self,
        now: DateTime<Utc>,
        timeout: std::time::Duration,
    ) -> Result<usize, IcsError> {
        let mut settings = self.settings().ok_or(IcsError::NotSet)?;
        let result = self.fetch_and_cache(&mut settings, now, timeout);
        match &result {
            Ok(_) => {
                settings.fetched_at = Some(now.timestamp());
                settings.last_error = None;
            }
            Err(e) => settings.last_error = Some(scrub(&e.to_string(), &settings.url)),
        }
        // A link cleared meanwhile stays cleared.
        if self.settings().is_some_and(|s| s.url == settings.url) {
            self.save_settings(&settings)?;
        }
        result
    }

    fn fetch_and_cache(
        &self,
        settings: &mut IcsSettings,
        now: DateTime<Utc>,
        timeout: std::time::Duration,
    ) -> Result<usize, IcsError> {
        let host = host(&settings.url);
        let cached_feed = std::fs::read_to_string(self.path(FEED_FILE)).ok();
        let agent = ureq::AgentBuilder::new().timeout(timeout).build();
        let mut request = agent.get(&settings.url);
        if cached_feed.is_some() {
            if let Some(etag) = &settings.etag {
                request = request.set("If-None-Match", etag);
            }
            if let Some(modified) = &settings.last_modified {
                request = request.set("If-Modified-Since", modified);
            }
        }
        let feed = match request.call() {
            Ok(response) if response.status() == 304 => cached_feed.unwrap_or_default(),
            Ok(response) => {
                let etag = response.header("ETag").map(str::to_string);
                let modified = response.header("Last-Modified").map(str::to_string);
                let mut body = String::new();
                response
                    .into_reader()
                    .take(MAX_FEED_BYTES)
                    .read_to_string(&mut body)
                    .map_err(|e| IcsError::Network {
                        host: host.clone(),
                        kind: e.kind().to_string(),
                    })?;
                write_private(&self.path(FEED_FILE), body.as_bytes())?;
                settings.etag = etag;
                settings.last_modified = modified;
                body
            }
            Err(ureq::Error::Status(304, _)) => cached_feed.unwrap_or_default(),
            Err(ureq::Error::Status(status, _)) => {
                return Err(IcsError::Http {
                    host,
                    status,
                    hint: match status {
                        401 | 403 | 404 | 410 => ": the link may have been reset. Copy the secret iCal address again and run voxtype meeting calendar set-ics.",
                        _ => "",
                    },
                });
            }
            Err(ureq::Error::Transport(t)) => {
                // Only the kind: the transport's own message names the link.
                return Err(IcsError::Network {
                    host,
                    kind: t.kind().to_string(),
                });
            }
        };
        let events =
            ics::events_between(&feed, now - Duration::days(1), now + AHEAD).map_err(|detail| {
                IcsError::NotCalendar {
                    host: host.clone(),
                    detail: scrub(&detail, &settings.url),
                }
            })?;
        let count = events.len();
        let cached = Cached {
            fetched_at: now.timestamp(),
            events,
        };
        let json = serde_json::to_vec(&cached).map_err(|e| IcsError::Io(e.to_string()))?;
        write_private(&self.path(EVENTS_FILE), &json)?;
        Ok(count)
    }
}

/// Write `bytes` to `path`, readable by the user alone, swapped in whole.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), IcsError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let io = |e: std::io::Error| IcsError::Io(format!("{}: {e}", path.display()));
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(io)?;
    let staged = dir.join(format!(
        ".{}.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("calendar"),
        std::process::id()
    ));
    let _ = std::fs::remove_file(&staged);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged)
        .map_err(io)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(io)?;
    std::fs::rename(&staged, path).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};

    const SECRET: &str = "/calendar/ical/pete%40example.com/private-0a1b2c3d4e5f/basic.ics";

    #[test]
    fn links_are_checked_and_only_their_host_is_shown() {
        assert_eq!(
            normalize_link(" webcal://p01-caldav.icloud.com/published/2/abc ").unwrap(),
            "https://p01-caldav.icloud.com/published/2/abc"
        );
        assert!(normalize_link("https://calendar.google.com/x.ics").is_ok());
        assert_eq!(normalize_link("ftp://x/y"), Err(IcsError::BadLink));
        assert_eq!(normalize_link("https:///nohost"), Err(IcsError::BadLink));
        assert_eq!(
            normalize_link("calendar.google.com"),
            Err(IcsError::BadLink)
        );
        let link = format!("https://calendar.google.com{SECRET}");
        assert_eq!(host(&link), "calendar.google.com");
        assert_eq!(host("https://user:pw@cal.example.com/x"), "cal.example.com");
        let scrubbed = scrub(&format!("GET {link} failed; path {SECRET}"), &link);
        assert!(!scrubbed.contains("private-0a1b2c3d4e5f"), "{scrubbed}");
        assert_eq!(scrubbed, "GET <calendar link> failed; path <calendar link>");
    }

    /// A one-shot-per-connection feed server. Each request is answered by
    /// `answer` from its headers; the headers are recorded.
    fn server(
        answer: impl Fn(&str) -> (u16, Vec<(&'static str, &'static str)>, String) + Send + 'static,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    head.push_str(&line);
                }
                log.lock().unwrap().push(head.clone());
                let (status, headers, body) = answer(&head);
                let mut reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                for (k, v) in headers {
                    reply.push_str(&format!("{k}: {v}\r\n"));
                }
                reply.push_str("\r\n");
                reply.push_str(&body);
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (base, seen)
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-12T13:55:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn the_feed_is_fetched_cached_privately_and_refetched_conditionally() {
        let (base, seen) = server(|head| {
            if head.contains("If-None-Match: \"v1\"") {
                (304, vec![], String::new())
            } else {
                (
                    200,
                    vec![
                        ("ETag", "\"v1\""),
                        ("Last-Modified", "Thu, 12 Nov 2026 09:00:00 GMT"),
                    ],
                    ics::tests::FIXTURE.to_string(),
                )
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("state/voxtype"));
        assert_eq!(
            store.refresh(now(), std::time::Duration::from_secs(5)),
            Err(IcsError::NotSet)
        );
        store.set_link(&format!("{base}{SECRET}")).unwrap();
        assert!(store.stale(now()));

        let n = store
            .refresh(now(), std::time::Duration::from_secs(5))
            .unwrap();
        // 12 Nov: the weekly 1:1, the New York review, the Outlook event.
        assert_eq!(n, 3);
        let settings = store.settings().unwrap();
        assert_eq!(settings.etag.as_deref(), Some("\"v1\""));
        assert_eq!(settings.fetched_at, Some(now().timestamp()));
        assert!(!store.stale(now()));
        for f in [SETTINGS_FILE, FEED_FILE, EVENTS_FILE] {
            let mode = std::fs::metadata(store.dir.join(f))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{f}");
        }
        let cached = store.cached().unwrap();
        let picked = ics::pick(&cached.events, now(), 10).unwrap();
        assert_eq!(picked.title, "Design review, with NY");

        // Not modified: the cached feed is parsed again, for the new window.
        let later = now() + Duration::minutes(20);
        store
            .refresh(later, std::time::Duration::from_secs(5))
            .unwrap();
        let heads = seen.lock().unwrap().clone();
        assert_eq!(heads.len(), 2);
        assert!(heads[1].contains("If-None-Match: \"v1\""), "{}", heads[1]);
        assert!(heads[1].contains("If-Modified-Since: Thu, 12 Nov 2026 09:00:00 GMT"));
        assert_eq!(store.cached().unwrap().fetched_at, later.timestamp());

        // Clearing forgets the link and everything fetched with it.
        assert!(store.clear());
        assert!(store.settings().is_none() && store.cached().is_none());
        assert!(!store.dir.join(FEED_FILE).exists());
        assert!(!store.clear());
    }

    #[test]
    fn failures_are_recorded_without_the_link() {
        let (base, _) = server(|head| {
            if head.contains("gone") {
                (404, vec![], String::new())
            } else {
                (200, vec![], "<html>Sign in</html>".to_string())
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let t = std::time::Duration::from_secs(5);

        store.set_link(&format!("{base}/gone{SECRET}")).unwrap();
        let err = store.refresh(now(), t).unwrap_err();
        assert!(matches!(err, IcsError::Http { status: 404, .. }), "{err}");
        let text = err.to_string();
        assert!(
            text.contains("127.0.0.1") && text.contains("reset"),
            "{text}"
        );
        assert!(!text.contains("private-0a1b2c3d4e5f"), "{text}");
        let recorded = store.settings().unwrap().last_error.unwrap();
        assert!(!recorded.contains("private-0a1b2c3d4e5f"), "{recorded}");

        store.set_link(&format!("{base}{SECRET}")).unwrap();
        let err = store.refresh(now(), t).unwrap_err();
        assert!(matches!(err, IcsError::NotCalendar { .. }), "{err}");
        assert!(!err.to_string().contains("private-0a1b2c3d4e5f"));
        assert!(store.cached().is_none(), "nothing cached from a bad feed");

        // Nobody listening: a network error naming the host only.
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        store
            .set_link(&format!("http://127.0.0.1:{port}{SECRET}"))
            .unwrap();
        let err = store.refresh(now(), t).unwrap_err();
        assert!(matches!(err, IcsError::Network { .. }), "{err}");
        assert!(!err.to_string().contains("private-0a1b2c3d4e5f"), "{err}");
        assert!(
            !format!("{err:?}").contains("private-0a1b2c3d4e5f"),
            "{err:?}"
        );
    }
}
