//! Which coding agent answers: resolution order and the agents voxtype knows.
//!
//! 1. `[meeting.assistant] agent` in config.
//! 2. `omarchy-default-agent`, when that command exists. Run with no
//!    arguments it only prints the chosen agent; with an argument it would
//!    change the default, so voxtype never passes one.
//! 3. Supported agent CLIs found on PATH.
//!
//! Nothing here needs Omarchy: without it, step 2 is skipped.

use super::consent::ConsentStore;
use super::AssistantEnv;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How safely an agent CLI can be run headless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// Every tool is off: text in, text out.
    ToolLess,
    /// The documented flags leave a sandboxed, read-only command tool that
    /// cannot be switched off. Used only when named explicitly (config or
    /// the Omarchy default), never picked from PATH on its own.
    ReadOnlySandbox,
    /// No documented way to run it without tools.
    Unsupported(&'static str),
}

/// An agent CLI voxtype knows about.
#[derive(Debug, Clone, Copy)]
pub struct AgentSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub binary: &'static str,
    pub support: Support,
}

impl AgentSpec {
    pub fn supported(&self) -> bool {
        !matches!(self.support, Support::Unsupported(_))
    }

    pub fn tool_less(&self) -> bool {
        self.support == Support::ToolLess
    }
}

/// Known agents, in PATH preference order.
pub const AGENTS: &[AgentSpec] = &[
    AgentSpec {
        id: "claude",
        name: "Claude",
        binary: "claude",
        support: Support::ToolLess,
    },
    AgentSpec {
        id: "codex",
        name: "Codex",
        binary: "codex",
        support: Support::ReadOnlySandbox,
    },
    AgentSpec {
        id: "hermes",
        name: "Hermes",
        binary: "hermes",
        support: Support::Unsupported(
            "its one-shot mode loads tools and auto-approves them, and no documented flag turns them all off",
        ),
    },
];

/// Look up an agent by id or a common alias (`claude-code`).
pub fn spec(id: &str) -> Option<&'static AgentSpec> {
    let id = id.trim().to_ascii_lowercase();
    let id = match id.as_str() {
        "claude-code" => "claude",
        other => other,
    };
    AGENTS.iter().find(|a| a.id == id)
}

/// The agent that will answer.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResolvedAgent {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    /// `config`, `omarchy` or `path`.
    pub source: &'static str,
}

/// One agent the UI can offer.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    /// Where the CLI is, if it was found.
    pub path: Option<PathBuf>,
    pub found: bool,
    pub supported: bool,
    /// Runs with every tool off. `false` for codex, which keeps a
    /// read-only sandboxed command tool.
    pub tool_less: bool,
    pub consent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consented_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Why it cannot be used, when it cannot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// How it was found: `config`, `omarchy`, `path`.
    pub sources: Vec<&'static str>,
}

/// Outcome of resolution, before consent is checked.
#[derive(Debug, Clone, Default)]
pub struct Resolution {
    pub agent: Option<ResolvedAgent>,
    pub candidates: Vec<Candidate>,
    /// What `omarchy-default-agent` printed, if it exists and printed.
    pub omarchy_default: Option<String>,
    /// Several agents could answer and none has consent yet: ask which.
    pub choice_required: bool,
    /// Why the configured agent cannot be used.
    pub problem: Option<String>,
}

impl Resolution {
    pub fn source(&self) -> &'static str {
        self.agent.as_ref().map(|a| a.source).unwrap_or("none")
    }
}

/// Find `binary` on the search PATH, then where it was when consent was given.
pub fn find_binary(
    env: &AssistantEnv,
    binary: &str,
    consent: &ConsentStore,
    id: &str,
) -> Option<PathBuf> {
    if let Some(path) = env.path.as_ref() {
        let cwd = std::env::temp_dir();
        if let Ok(found) = which::which_in(binary, Some(path), cwd) {
            return Some(found);
        }
    }
    consent
        .get(id)
        .and_then(|e| e.path.clone())
        .filter(|p| is_executable(p))
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Ask `omarchy-default-agent` for the default agent. `None` when the
/// command is missing, fails or prints nothing (no default chosen).
pub fn omarchy_default_agent(env: &AssistantEnv) -> Option<String> {
    let path = env.path.as_ref()?;
    let exe = which::which_in(&env.omarchy_command, Some(path), std::env::temp_dir()).ok()?;
    let mut child = std::process::Command::new(exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    use std::io::Read;
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let id = out.lines().next().unwrap_or("").trim().to_string();
    (!id.is_empty()).then_some(id)
}

/// Resolve the agent: config, then the Omarchy default, then PATH.
pub fn resolve(configured: Option<&str>, env: &AssistantEnv, consent: &ConsentStore) -> Resolution {
    let mut res = Resolution::default();

    let add = |res: &mut Resolution, id: &str, source: &'static str| {
        if let Some(c) = res.candidates.iter_mut().find(|c| c.id == id) {
            if !c.sources.contains(&source) {
                c.sources.push(source);
            }
            return;
        }
        let known = spec(id);
        let path = known.and_then(|s| find_binary(env, s.binary, consent, s.id));
        let entry = consent.get(known.map(|s| s.id).unwrap_or(id));
        let note = match known {
            None => Some(format!(
                "not supported (supported: {})",
                super::SUPPORTED_AGENTS.join(", ")
            )),
            Some(s) => match s.support {
                Support::Unsupported(why) => Some(format!("not supported: {why}")),
                _ if path.is_none() => Some(format!("{} was not found on PATH", s.binary)),
                Support::ReadOnlySandbox => Some(
                    "runs in a read-only sandbox; its command tool cannot be turned off".into(),
                ),
                Support::ToolLess => None,
            },
        };
        res.candidates.push(Candidate {
            id: known.map(|s| s.id).unwrap_or(id).to_string(),
            name: known
                .map(|s| s.name.to_string())
                .unwrap_or_else(|| id.to_string()),
            found: path.is_some(),
            path,
            supported: known.is_some_and(|s| s.supported()),
            tool_less: known.is_some_and(|s| s.tool_less()),
            consent: entry.is_some(),
            consented_at: entry.map(|e| e.allowed_at),
            note,
            sources: vec![source],
        });
    };

    let usable = |c: &Candidate| c.supported && c.found;
    let pick = |c: &Candidate, source: &'static str| ResolvedAgent {
        id: c.id.clone(),
        name: c.name.clone(),
        path: c.path.clone().unwrap_or_default(),
        source,
    };

    // 1. Config.
    if let Some(id) = configured.map(str::trim).filter(|s| !s.is_empty()) {
        add(&mut res, id, "config");
        let c = res.candidates.last().cloned().expect("just added");
        if usable(&c) {
            res.agent = Some(pick(&c, "config"));
        } else {
            res.problem = Some(format!(
                "[meeting.assistant] agent = \"{}\": {}",
                id,
                c.note.clone().unwrap_or_else(|| "cannot be used".into())
            ));
        }
    }

    // 2. The Omarchy default.
    if let Some(id) = omarchy_default_agent(env) {
        res.omarchy_default = Some(id.clone());
        add(&mut res, &id, "omarchy");
        let c = res
            .candidates
            .iter()
            .find(|c| spec(&id).map(|s| s.id).unwrap_or(&id) == c.id)
            .cloned();
        if res.agent.is_none() && res.problem.is_none() {
            if let Some(c) = c.filter(usable) {
                res.agent = Some(pick(&c, "omarchy"));
            }
        }
    }

    // 3. PATH. Every known CLI found is listed so the UI can offer it.
    for s in AGENTS {
        if env
            .path
            .as_ref()
            .is_some_and(|p| which::which_in(s.binary, Some(p), std::env::temp_dir()).is_ok())
        {
            add(&mut res, s.id, "path");
        }
    }
    if res.agent.is_none() && res.problem.is_none() {
        // Only tool-less agents are picked on their own.
        let auto: Vec<&Candidate> = res
            .candidates
            .iter()
            .filter(|c| usable(c) && c.tool_less && c.sources.contains(&"path"))
            .collect();
        let chosen = auto
            .iter()
            .find(|c| c.consent)
            .or_else(|| auto.first())
            .map(|c| pick(c, "path"));
        res.choice_required = auto.len() > 1 && !auto.iter().any(|c| c.consent);
        res.agent = chosen;
    }

    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn script(dir: &Path, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new(bins: &[&str]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir(dir.path().join("bin")).unwrap();
            for b in bins {
                script(&dir.path().join("bin"), b, "exit 0");
            }
            Self { dir }
        }

        fn omarchy(&self, prints: &str) {
            script(
                &self.dir.path().join("bin"),
                "omarchy-default-agent",
                &format!("[ $# -eq 0 ] || exit 9\necho '{prints}'"),
            );
        }

        fn env(&self) -> AssistantEnv {
            AssistantEnv {
                path: Some(OsString::from(self.dir.path().join("bin"))),
                consent_file: self.dir.path().join("consent.json"),
                omarchy_command: "omarchy-default-agent".into(),
            }
        }

        fn consent(&self) -> ConsentStore {
            ConsentStore::load(&self.env().consent_file)
        }

        fn resolve(&self, configured: Option<&str>) -> Resolution {
            resolve(configured, &self.env(), &self.consent())
        }
    }

    fn ids(r: &Resolution) -> Vec<&str> {
        r.candidates.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn config_wins_over_omarchy_and_path() {
        let f = Fixture::new(&["claude", "codex"]);
        f.omarchy("claude");
        let r = f.resolve(Some("codex"));
        let a = r.agent.unwrap();
        assert_eq!((a.id.as_str(), a.source), ("codex", "config"));
        assert_eq!(r.omarchy_default.as_deref(), Some("claude"));
    }

    #[test]
    fn omarchy_default_beats_path() {
        let f = Fixture::new(&["claude", "codex"]);
        f.omarchy("codex");
        let a = f.resolve(None).agent.unwrap();
        assert_eq!((a.id.as_str(), a.source), ("codex", "omarchy"));
    }

    #[test]
    fn without_omarchy_path_is_used() {
        let f = Fixture::new(&["claude"]);
        let r = f.resolve(None);
        assert_eq!(r.omarchy_default, None);
        let a = r.agent.unwrap();
        assert_eq!((a.id.as_str(), a.source), ("claude", "path"));
        assert!(a.path.ends_with("bin/claude"));
    }

    #[test]
    fn an_unsupported_omarchy_default_falls_through_to_path() {
        let f = Fixture::new(&["claude", "hermes"]);
        f.omarchy("hermes");
        let r = f.resolve(None);
        assert_eq!(r.agent.unwrap().id, "claude");
        let hermes = r.candidates.iter().find(|c| c.id == "hermes").unwrap();
        assert!(!hermes.supported);
        assert!(hermes.found);
        assert_eq!(hermes.sources, vec!["omarchy", "path"]);
    }

    #[test]
    fn codex_alone_on_path_is_offered_but_not_picked() {
        let f = Fixture::new(&["codex"]);
        let r = f.resolve(None);
        assert!(r.agent.is_none());
        assert_eq!(ids(&r), vec!["codex"]);
        assert!(!r.candidates[0].tool_less);
    }

    #[test]
    fn several_candidates_are_all_listed() {
        let f = Fixture::new(&["claude", "codex", "hermes"]);
        let r = f.resolve(None);
        assert_eq!(ids(&r), vec!["claude", "codex", "hermes"]);
        assert_eq!(r.agent.unwrap().id, "claude");
        assert!(!r.choice_required);
    }

    #[test]
    fn nothing_found_resolves_nothing() {
        let f = Fixture::new(&[]);
        let r = f.resolve(None);
        assert!(r.agent.is_none());
        assert!(r.candidates.is_empty());
        assert_eq!(r.source(), "none");
    }

    #[test]
    fn a_configured_agent_that_is_missing_or_unknown_is_a_problem() {
        let f = Fixture::new(&["claude"]);
        let r = f.resolve(Some("codex"));
        assert!(r.agent.is_none());
        assert!(r.problem.unwrap().contains("codex was not found"));

        let r = f.resolve(Some("opencode"));
        assert!(r.problem.unwrap().contains("not supported"));

        let r = f.resolve(Some("hermes"));
        assert!(r.problem.unwrap().contains("not supported"));
    }

    #[test]
    fn consent_path_finds_an_agent_off_path() {
        let f = Fixture::new(&[]);
        let elsewhere = f.dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        script(&elsewhere, "claude", "exit 0");
        f.consent()
            .allow("claude", Some(elsewhere.join("claude")))
            .unwrap();
        let r = f.resolve(Some("claude"));
        let a = r.agent.unwrap();
        assert_eq!(a.path, elsewhere.join("claude"));
        assert!(r.candidates[0].consent);
    }

    #[test]
    fn omarchy_is_never_given_arguments() {
        let f = Fixture::new(&["claude"]);
        // The fixture script exits 9 (printing nothing) if given arguments.
        f.omarchy("claude");
        assert_eq!(f.resolve(None).omarchy_default.as_deref(), Some("claude"));
    }

    #[test]
    fn aliases_resolve() {
        assert_eq!(spec("Claude-Code").unwrap().id, "claude");
        assert!(spec("opencode").is_none());
    }
}
