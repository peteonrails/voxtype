//! Meeting assistant: ask a meeting a question, and summarize it, through
//! the user's own coding agent run headless with no tools.
//!
//! - [`agent`] resolves which agent answers: `[meeting.assistant] agent`,
//!   then `omarchy-default-agent`, then supported CLIs on PATH.
//! - [`consent`] keeps the per-agent "send transcripts to X" answer.
//! - [`runner`] runs the agent's print mode with tools off.
//! - [`local`] is the fallback: the Ollama model from `[meeting.summary]`.
//! - `[meeting.summary] backend = "omarchy"` (or `"openai"` on this
//!   machine) answers through [`crate::llm`] instead, ahead of any agent and
//!   without agent consent, since the transcript stays on the machine.
//! - [`prompt`] builds the prompt shared by ask and summary.
//! - [`asks`] stores each question and answer in `asks.jsonl`.
//! - [`summarize`] writes the meeting summary, by hand or at stop.

pub mod agent;
pub mod asks;
pub mod consent;
pub mod jail;
pub mod local;
#[cfg(target_os = "linux")]
pub mod notify;
pub mod prompt;
pub mod runner;
pub mod summarize;
#[cfg(test)]
pub(crate) mod testing;

use crate::meeting::data::MeetingData;
use agent::{Resolution, ResolvedAgent};
use chrono::Utc;
use consent::ConsentStore;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

/// Agent ids the assistant can run headless.
pub const SUPPORTED_AGENTS: &[&str] = &["claude", "codex", "pi"];

/// Running summary of a long meeting's earlier part, so repeated asks
/// during a meeting only summarize what is new.
pub const CONTEXT_CACHE_FILE: &str = "assistant-context.json";

#[derive(Error, Debug)]
pub enum AssistantError {
    #[error(
        "Meeting transcripts have not been allowed to go to {name}.\n  \
         Allow it once with: voxtype meeting assistant allow {agent}\n  \
         Or answer with the local model: add --local, or set [meeting.assistant] local_only = true"
    )]
    ConsentRequired { agent: String, name: String },

    #[error("{0}\n  Supported agents: claude, codex, pi. See: voxtype meeting assistant status")]
    AgentUnavailable(String),

    #[error("Agent '{0}' cannot be run without tools, so voxtype does not use it")]
    UnsupportedAgent(String),

    #[error("{name} failed: {message}")]
    Agent { name: String, message: String },

    #[error(
        "{name} did not answer within {secs}s. Raise [meeting.assistant] timeout_secs for long meetings."
    )]
    Timeout { name: String, secs: u64 },

    /// The message names the backend: "Ollama (llama3:latest at
    /// localhost:11434) returned an error: ...".
    #[error("{0}")]
    Local(String),

    #[error(
        "Answers and summaries need an agent or a local model.\n  \
         No supported agent CLI (claude, codex, pi) was found, and the local model failed: {message}\n  \
         Install Claude Code, Codex or Pi, or run Ollama with: ollama pull {model}"
    )]
    NoBackend { message: String, model: String },

    #[error("The meeting has no transcript yet")]
    EmptyTranscript,

    #[error("{0}")]
    Storage(String),

    #[error("Could not read the summary the model wrote: {0}")]
    Parse(String),
}

impl AssistantError {
    /// Stable code for `--json` error events and the UI.
    pub fn code(&self) -> &'static str {
        match self {
            AssistantError::ConsentRequired { .. } => "consent_required",
            AssistantError::AgentUnavailable(_) | AssistantError::UnsupportedAgent(_) => {
                "agent_unavailable"
            }
            AssistantError::Agent { .. } => "agent_failed",
            AssistantError::Timeout { .. } => "timeout",
            AssistantError::Local(_) => "local_failed",
            AssistantError::NoBackend { .. } => "no_backend",
            AssistantError::EmptyTranscript => "empty_transcript",
            AssistantError::Storage(_) => "storage",
            AssistantError::Parse(_) => "parse",
        }
    }
}

/// Where the assistant looks for things. Replaced in tests.
#[derive(Debug, Clone)]
pub struct AssistantEnv {
    /// Directories searched for agent CLIs and `omarchy-default-agent`.
    pub path: Option<OsString>,
    pub consent_file: PathBuf,
    pub omarchy_command: String,
    /// codex's CODEX_HOME, where its `auth.json` is.
    pub codex_home: PathBuf,
    /// Where Omarchy's local text model is found.
    pub omarchy: crate::llm::omarchy::OmarchyEnv,
}

impl AssistantEnv {
    pub fn from_process() -> Self {
        Self {
            path: std::env::var_os("PATH"),
            consent_file: crate::config::Config::state_dir().join(consent::CONSENT_FILE),
            omarchy_command: "omarchy-default-agent".to_string(),
            codex_home: std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
                .unwrap_or_else(|| PathBuf::from(".codex")),
            omarchy: crate::llm::omarchy::OmarchyEnv::from_process(),
        }
    }
}

/// The assistant's settings, from `[meeting.assistant]` and `[meeting.summary]`.
#[derive(Debug, Clone)]
pub struct AssistantSettings {
    pub agent: Option<String>,
    pub local_only: bool,
    pub timeout: Duration,
    pub max_transcript_chars: usize,
    /// As configured: `None` means the default (open the meeting view),
    /// an empty string means no Open action. See [`open_action_command`].
    pub open_command: Option<String>,
    pub summary_auto: Option<bool>,
    /// `[meeting.summary] backend`: `local` and `remote` keep the older
    /// summarizers for `meeting summarize` and auto-summary.
    pub summary_backend: String,
    pub summary: crate::meeting::summary::SummaryConfig,
    /// `[meeting.summary] backend = "omarchy"` or `"openai"`.
    pub llm: Option<crate::llm::LlmConfig>,
    pub env: AssistantEnv,
    /// `[meeting] my_name`: who the user is, shown for "You" in prompts.
    pub my_name: Option<String>,
}

impl AssistantSettings {
    pub fn from_config(cfg: &crate::config::Config) -> Self {
        let a = &cfg.meeting.assistant;
        let s = &cfg.meeting.summary;
        Self {
            agent: a.agent.clone().filter(|s| !s.trim().is_empty()),
            local_only: a.local_only,
            timeout: Duration::from_secs(a.timeout_secs.max(1)),
            max_transcript_chars: (a.max_transcript_chars as usize).max(2_000),
            open_command: a.open_command.clone(),
            summary_auto: s.auto,
            summary_backend: s.backend.clone(),
            summary: crate::meeting::summary::SummaryConfig {
                backend: s.backend.clone(),
                ollama_url: s.ollama_url.clone(),
                ollama_model: s.ollama_model.clone(),
                remote_endpoint: s.remote_endpoint.clone(),
                remote_api_key: s.remote_api_key.clone(),
                timeout_secs: s.timeout_secs,
            },
            llm: s.llm(),
            env: AssistantEnv::from_process(),
            my_name: cfg
                .meeting
                .my_name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string),
        }
    }

    pub fn consent(&self) -> ConsentStore {
        ConsentStore::load(&self.env.consent_file)
    }

    /// The chosen LLM backend, when `[meeting.summary] backend` names one.
    /// `None` when none is chosen, or when `require_local` and the server
    /// is not on this machine. Asking Omarchy may take a moment: it runs
    /// `omarchy-models status --json`.
    pub fn llm_backend(
        &self,
        require_local: bool,
    ) -> Option<Result<Backend, crate::llm::LlmError>> {
        let config = self.llm.as_ref()?;
        match config.resolve(&self.env.omarchy) {
            Ok(endpoint) if require_local && !endpoint.is_local() => None,
            Ok(endpoint) => Some(Ok(Backend::Llm(endpoint))),
            // A server that is not on this machine is no concern of a
            // caller that needs a local one, even when misconfigured.
            Err(_)
                if require_local
                    && config.provider == crate::llm::Provider::OpenAi
                    && !config
                        .url
                        .as_deref()
                        .is_some_and(crate::llm::is_loopback_url) =>
            {
                None
            }
            Err(e) => Some(Err(e)),
        }
    }

    pub fn resolve(&self, agent_override: Option<&str>) -> Resolution {
        agent::resolve(
            agent_override.or(self.agent.as_deref()),
            &self.env,
            &self.consent(),
        )
    }

    /// Is auto-summary on? Unset means on once the resolved agent has
    /// consent, and never when only the local model may be used.
    pub fn auto_summary(&self) -> bool {
        match self.summary_auto {
            Some(on) => on,
            None => {
                !self.local_only
                    && self
                        .resolve(None)
                        .agent
                        .is_some_and(|a| self.consent().is_allowed(&a.id))
            }
        }
    }
}

/// The command the Open action of an `ask --notify` notification runs.
///
/// An explicit `open_command` wins, and an empty one turns Open off. Unset,
/// Open raises the meeting view on its Ask tab through `voxtype` itself:
/// `exe` (the running binary, not whatever `voxtype` PATH finds), when
/// `has_view` says that binary has a `meeting view` command.
pub fn open_action_command(
    configured: Option<&str>,
    exe: Option<&Path>,
    has_view: impl FnOnce(&Path) -> bool,
) -> Option<String> {
    match configured {
        Some(cmd) if cmd.trim().is_empty() => None,
        Some(cmd) => Some(cmd.to_string()),
        None => {
            let exe = exe?;
            has_view(exe).then(|| {
                format!(
                    "{} meeting view \"$VOXTYPE_MEETING_ID\" --tab ask",
                    shell_quote(&exe.display().to_string())
                )
            })
        }
    }
}

/// Does `exe` have the `meeting view` command? Older and parallel builds
/// do not; asking its own help is the one reliable test.
pub fn binary_has_meeting_view(exe: &Path) -> bool {
    agent::run_quick(exe, &["meeting", "view", "--help"]).is_some()
}

/// Quote `s` for `sh -c`.
fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+=:,@".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// What answers: an agent CLI, the local model, or a chat endpoint.
#[derive(Debug, Clone, PartialEq)]
pub enum Backend {
    Agent(ResolvedAgent),
    /// `[meeting.summary] backend = "omarchy"` or `"openai"`.
    Llm(crate::llm::Endpoint),
    Local {
        url: String,
        model: String,
        /// Chosen because no agent was found, not by request.
        fallback: bool,
    },
}

impl Backend {
    /// `claude`, `codex`, `local:<model>`.
    pub fn id(&self) -> String {
        match self {
            Backend::Agent(a) => a.id.clone(),
            Backend::Llm(e) => e.id(),
            Backend::Local { model, .. } => format!("local:{model}"),
        }
    }

    /// "Claude", "Local (llama3.2)".
    pub fn display_name(&self) -> String {
        match self {
            Backend::Agent(a) => a.name.clone(),
            Backend::Llm(e) => e.display_name(),
            Backend::Local { model, .. } => format!("Local ({model})"),
        }
    }

    /// Run `prompt`, streaming the answer to `on_text`.
    pub fn complete(
        &self,
        prompt: &str,
        json: bool,
        timeout: Duration,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<String, AssistantError> {
        match self {
            Backend::Agent(a) => runner::run_agent(a, prompt, timeout, on_text),
            // The prompt asks for JSON already; a `response_format` field is
            // not sent, since not every local runtime takes one.
            Backend::Llm(endpoint) => crate::llm::chat(
                endpoint,
                &crate::llm::ChatRequest {
                    system: runner::SYSTEM_PROMPT,
                    user: prompt,
                    timeout,
                    stream: true,
                },
                on_text,
            )
            .map_err(|e| match e {
                crate::llm::LlmError::Timeout { label, secs } => {
                    AssistantError::Timeout { name: label, secs }
                }
                other => AssistantError::Local(other.to_string()),
            }),
            Backend::Local {
                url,
                model,
                fallback,
            } => local::generate(url, model, prompt, json, timeout, on_text).map_err(|e| {
                match (fallback, e) {
                    (true, AssistantError::Local(message)) => AssistantError::NoBackend {
                        message,
                        model: model.clone(),
                    },
                    (_, e) => e,
                }
            }),
        }
    }
}

/// Per-call overrides from the command line.
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    /// `--local`: use the local model.
    pub local: bool,
    /// `--agent <id>`: use this agent instead of the configured one.
    pub agent: Option<String>,
}

/// Pick the backend for one ask. Never asks the user; a TTY prompt for
/// consent is the caller's business.
///
/// One order for asks and summaries (see [`choose_summary_backend`]):
///
/// 0. Unless `--agent` names an agent: `[meeting.summary] backend =
///    "omarchy"` (or `"openai"` on this machine) when its model answers.
///    It needs no consent, since the transcript stays on the machine. When
///    Omarchy has no model running, the log says how to start one and the
///    order below decides as if it were not chosen.
/// 1. `local_only` or `--local`: the local model.
/// 2. The agent (`--agent`, `[meeting.assistant] agent`, the Omarchy
///    default, or one on PATH), when it has consent. Allowing an agent is
///    the newest and most explicit choice, so it wins over an older
///    `[meeting.summary] backend`.
/// 3. `[meeting.summary] backend = "local"`, for someone who never allowed
///    an agent: they chose Ollama before the assistant existed.
/// 4. Otherwise: an agent without consent needs it, and with no agent at all
///    the local model is the fallback.
pub fn choose_backend(
    settings: &AssistantSettings,
    overrides: &Overrides,
) -> Result<Backend, AssistantError> {
    if overrides.agent.is_none() {
        match settings.llm_backend(true) {
            Some(Ok(backend)) => return Ok(backend),
            Some(Err(e)) => tracing::warn!("{e} Answering the usual way instead."),
            None => {}
        }
    }
    choose_backend_without_llm(settings, overrides)
}

/// [`choose_backend`] from step 1: the order before the LLM backends.
fn choose_backend_without_llm(
    settings: &AssistantSettings,
    overrides: &Overrides,
) -> Result<Backend, AssistantError> {
    let local = |fallback| Backend::Local {
        url: settings.summary.ollama_url.clone(),
        model: settings.summary.ollama_model.clone(),
        fallback,
    };
    if settings.local_only || overrides.local {
        return Ok(local(false));
    }
    let res = settings.resolve(overrides.agent.as_deref());
    if let Some(agent) = res.agent {
        if settings.consent().is_allowed(&agent.id) {
            return Ok(Backend::Agent(agent));
        }
        if overrides.agent.is_none() && settings.summary_backend == "local" {
            return Ok(local(false));
        }
        return Err(AssistantError::ConsentRequired {
            agent: agent.id,
            name: agent.name,
        });
    }
    if overrides.agent.is_none() && settings.summary_backend == "local" && res.problem.is_none() {
        return Ok(local(false));
    }
    if let Some(problem) = res.problem {
        return Err(AssistantError::AgentUnavailable(problem));
    }
    Ok(local(true))
}

/// What a summary runs on.
#[derive(Debug, Clone, PartialEq)]
pub enum SummaryBackend {
    /// An agent or the local model, the same as `ask` would use.
    Assistant(Backend),
    /// `[meeting.summary] backend = "local"` or `"remote"`: the summarizers
    /// from before the assistant, kept for people who never allowed an agent.
    Legacy(String),
}

impl SummaryBackend {
    /// `claude`, `local:<model>`, `ollama`, `remote`.
    pub fn id(&self) -> String {
        match self {
            SummaryBackend::Assistant(b) => b.id(),
            SummaryBackend::Legacy(kind) if kind == "local" => "ollama".to_string(),
            SummaryBackend::Legacy(kind) => kind.clone(),
        }
    }

    /// `agent`, `local` or `remote`, for `assistant status --json`.
    pub fn kind(&self) -> &'static str {
        match self {
            SummaryBackend::Assistant(Backend::Agent(_)) => "agent",
            SummaryBackend::Assistant(Backend::Llm(e)) if e.is_local() => "local",
            SummaryBackend::Assistant(Backend::Llm(_)) => "remote",
            SummaryBackend::Assistant(Backend::Local { .. }) => "local",
            SummaryBackend::Legacy(kind) if kind == "local" => "local",
            SummaryBackend::Legacy(_) => "remote",
        }
    }

    /// "Claude", "Local (llama3:latest)", "Remote (api.example.com)".
    pub fn display_name(&self, settings: &AssistantSettings) -> String {
        match self {
            SummaryBackend::Assistant(b) => b.display_name(),
            SummaryBackend::Legacy(kind) if kind == "local" => {
                format!("Local ({})", settings.summary.ollama_model)
            }
            SummaryBackend::Legacy(_) => {
                let endpoint = settings.summary.remote_endpoint.as_deref().unwrap_or("");
                let host = endpoint
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .split('/')
                    .next()
                    .unwrap_or("");
                format!("Remote ({host})")
            }
        }
    }
}

/// Pick the backend for one summary, automatic or asked for, in the order
/// [`choose_backend`] describes. The difference is `[meeting.summary]
/// backend = "remote"`, which only summaries can use, and that `"local"`
/// keeps its own summarizer for summaries.
pub fn choose_summary_backend(
    settings: &AssistantSettings,
    overrides: &Overrides,
) -> Result<SummaryBackend, AssistantError> {
    if overrides.agent.is_none() {
        // A summary may go to a remote "openai" server, unless only local
        // models may be used.
        let require_local = settings.local_only || overrides.local;
        match settings.llm_backend(require_local) {
            Some(Ok(backend)) => return Ok(SummaryBackend::Assistant(backend)),
            Some(Err(e)) => tracing::warn!("{e} Summarizing the usual way instead."),
            None => {}
        }
    }
    summary_backend_without_llm(settings, overrides)
}

/// [`choose_summary_backend`] without the LLM backends: what a summary
/// falls back to when the chosen model is not running or fails.
pub fn summary_backend_without_llm(
    settings: &AssistantSettings,
    overrides: &Overrides,
) -> Result<SummaryBackend, AssistantError> {
    if settings.local_only || overrides.local || overrides.agent.is_some() {
        return choose_backend_without_llm(settings, overrides).map(SummaryBackend::Assistant);
    }
    let res = settings.resolve(None);
    if let Some(agent) = res.agent {
        if settings.consent().is_allowed(&agent.id) {
            return Ok(SummaryBackend::Assistant(Backend::Agent(agent)));
        }
    }
    if matches!(settings.summary_backend.as_str(), "local" | "remote") {
        return Ok(SummaryBackend::Legacy(settings.summary_backend.clone()));
    }
    choose_backend_without_llm(settings, overrides).map(SummaryBackend::Assistant)
}

/// The meeting's folder, when it has one.
pub fn meeting_folder(meeting: &MeetingData) -> Option<&Path> {
    meeting.metadata.storage_path.as_deref()
}

/// The user's notes for the meeting, as storage loaded them (`notes.md`).
pub fn read_notes(meeting: &MeetingData) -> Option<String> {
    meeting
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(String::from)
}

/// The annotations on transcript lines from `from_ms` on, in transcript order.
pub fn read_annotations(meeting: &MeetingData, from_ms: u64) -> Vec<prompt::Annotation> {
    meeting
        .transcript
        .segments
        .iter()
        .filter(|s| s.start_ms >= from_ms)
        .filter_map(|s| {
            s.annotation.as_ref().map(|a| prompt::Annotation {
                start_ms: s.start_ms,
                text: crate::meeting::export::annotation_line(a),
            })
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContextCache {
    backend: String,
    through_ms: u64,
    summary: String,
}

/// Build the prompt context for `meeting`. `last` limits it to the final
/// stretch of the transcript. A meeting too long for the budget gets a
/// running summary of its earlier part, made with `backend` and cached in
/// the meeting folder.
pub fn build_context(
    settings: &AssistantSettings,
    meeting: &MeetingData,
    last: Option<Duration>,
    backend: &Backend,
) -> Result<prompt::Context, AssistantError> {
    // Storage already shows my_name for "You"; applying it again here keeps
    // a caller that loaded the meeting some other way consistent.
    let shown;
    let meeting = match settings.my_name.as_deref() {
        Some(name) => {
            let mut m = meeting.clone();
            m.transcript.apply_my_name(name);
            shown = m;
            &shown
        }
        None => meeting,
    };
    let header = prompt::header(meeting, Utc::now());
    let all = prompt::lines(&meeting.transcript.segments);
    let mut ctx = prompt::Context {
        header,
        reader: prompt::reader_line(&meeting.transcript.segments, settings.my_name.as_deref()),
        notes: read_notes(meeting),
        annotations: read_annotations(meeting, 0),
        ..Default::default()
    };
    let budget = settings.max_transcript_chars;

    if let Some(window) = last {
        let end = prompt::transcript_end_ms(meeting);
        let lines = prompt::last_window(&all, window, end);
        let from = lines.first().map(|l| l.start_ms).unwrap_or(end);
        ctx.excerpt = Some(format!(
            "Excerpt: only the last {} of the transcript, from [{}] to [{}].",
            human_duration(window),
            prompt::ts(from),
            prompt::ts(end)
        ));
        ctx.lines = match prompt::fit(lines, budget) {
            prompt::Fit::Whole(l) => l,
            prompt::Fit::Split { recent, .. } => recent,
        };
        ctx.annotations = read_annotations(meeting, from);
        return Ok(ctx);
    }

    match prompt::fit(all, budget) {
        prompt::Fit::Whole(lines) => ctx.lines = lines,
        prompt::Fit::Split { earlier, recent } => {
            let through = recent.first().map(|l| l.start_ms).unwrap_or(0);
            let summary =
                earlier_summary(settings, meeting, &ctx.header, &earlier, through, backend)?;
            ctx.earlier_summary = Some((summary, through));
            ctx.lines = recent;
        }
    }
    Ok(ctx)
}

/// Summarize `earlier` (everything before `through`), reusing and extending
/// a cached running summary when one exists for the same backend.
fn earlier_summary(
    settings: &AssistantSettings,
    meeting: &MeetingData,
    header: &str,
    earlier: &[prompt::Line],
    through: u64,
    backend: &Backend,
) -> Result<String, AssistantError> {
    let cache_path = meeting_folder(meeting).map(|f| f.join(CONTEXT_CACHE_FILE));
    let cached: Option<ContextCache> = cache_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(|c: &ContextCache| c.backend == backend.id() && c.through_ms <= through);

    let (mut summary, new_lines): (Option<String>, Vec<prompt::Line>) = match &cached {
        Some(c) => (
            Some(c.summary.clone()),
            earlier
                .iter()
                .filter(|l| l.start_ms >= c.through_ms)
                .cloned()
                .collect(),
        ),
        None => (None, earlier.to_vec()),
    };
    if new_lines.is_empty() {
        if let Some(s) = summary {
            return Ok(s);
        }
    }
    let piece_budget = settings
        .max_transcript_chars
        .saturating_sub(4_000)
        .max(1_000);
    for piece in prompt::pieces(&new_lines, piece_budget) {
        let p = prompt::rolling_summary_prompt(header, summary.as_deref(), &piece);
        summary = Some(backend.complete(&p, false, settings.timeout, &mut |_| {})?);
    }
    let summary = summary.unwrap_or_default();
    if let Some(path) = cache_path {
        let cache = ContextCache {
            backend: backend.id(),
            through_ms: through,
            summary: summary.clone(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&cache) {
            let _ = std::fs::write(path, json);
        }
    }
    Ok(summary)
}

/// "5 minutes", "90 seconds", "1h 30m".
pub fn human_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs.is_multiple_of(3600) && secs >= 3600 {
        let h = secs / 3600;
        format!("{h} hour{}", if h == 1 { "" } else { "s" })
    } else if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else if secs.is_multiple_of(60) && secs > 0 {
        let m = secs / 60;
        format!("{m} minute{}", if m == 1 { "" } else { "s" })
    } else {
        format!("{secs} seconds")
    }
}

/// Parse `--last`: `5m`, `90s`, `1h`, `2m30s`, or a bare number of minutes.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim().to_ascii_lowercase();
    if s.is_empty() {
        return Err("empty duration".into());
    }
    if let Ok(mins) = s.parse::<u64>() {
        return Ok(Duration::from_secs(mins * 60));
    }
    let mut total = 0u64;
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: u64 = num
            .parse()
            .map_err(|_| format!("'{s}' is not a duration like 5m, 90s or 1h"))?;
        num.clear();
        total += match c {
            'h' => n * 3600,
            'm' => n * 60,
            's' => n,
            _ => return Err(format!("'{s}' is not a duration like 5m, 90s or 1h")),
        };
    }
    if !num.is_empty() || total == 0 {
        return Err(format!("'{s}' is not a duration like 5m, 90s or 1h"));
    }
    Ok(Duration::from_secs(total))
}

/// One question about a meeting.
#[derive(Debug, Clone)]
pub struct AskRequest {
    pub question: String,
    pub last: Option<Duration>,
}

/// Ask `meeting` a question with `backend`, streaming the answer to
/// `on_text`, and record it in the meeting's `asks.jsonl`.
pub fn ask(
    settings: &AssistantSettings,
    meeting: &MeetingData,
    request: &AskRequest,
    backend: &Backend,
    on_text: &mut dyn FnMut(&str),
) -> Result<asks::AskRecord, AssistantError> {
    let ctx = build_context(settings, meeting, request.last, backend)?;
    let prompt = ctx.ask(&request.question);
    let answer = backend.complete(&prompt, false, settings.timeout, on_text)?;
    let record = asks::AskRecord {
        id: uuid::Uuid::new_v4().to_string(),
        time: Utc::now(),
        question: request.question.trim().to_string(),
        citations: prompt::citations(&answer),
        answer,
        agent: backend.id(),
        agent_name: backend.display_name(),
        last_secs: request.last.map(|d| d.as_secs()),
    };
    if let Some(folder) = meeting_folder(meeting) {
        if let Err(e) = asks::append(folder, &record) {
            tracing::warn!("Could not save the answer to {}: {}", asks::ASKS_FILE, e);
        }
    }
    Ok(record)
}

/// `voxtype meeting assistant status --json`.
pub fn status_json(settings: &AssistantSettings) -> serde_json::Value {
    let consent = settings.consent();
    let res = settings.resolve(None);
    // Omarchy is asked once here, for both answers below.
    let llm = settings
        .llm
        .as_ref()
        .map(|c| (c, c.resolve(&settings.env.omarchy)));
    let llm_ask = llm.as_ref().and_then(|(_, r)| match r {
        Ok(e) if e.is_local() => Some(Backend::Llm(e.clone())),
        _ => None,
    });
    let backend = match llm_ask.clone() {
        Some(b) => Ok(b),
        None => choose_backend_without_llm(settings, &Overrides::default()),
    };
    let (backend_kind, blocked) = match &backend {
        Ok(Backend::Agent(_)) => ("agent", None),
        Ok(Backend::Llm(e)) if !e.is_local() => ("remote", None),
        Ok(Backend::Local { .. } | Backend::Llm(_)) => ("local", None),
        Err(e) => ("none", Some(e.code())),
    };
    let agent = res.agent.as_ref();
    let llm_summary = llm.as_ref().and_then(|(_, r)| match r {
        Ok(e) if e.is_local() || !settings.local_only => Some(Backend::Llm(e.clone())),
        _ => None,
    });
    let summary = match llm_summary {
        Some(b) => Ok(SummaryBackend::Assistant(b)),
        None => summary_backend_without_llm(settings, &Overrides::default()),
    };
    let llm_status = llm.as_ref().map(|(c, r)| {
        serde_json::json!({
            "provider": c.provider.as_str(),
            "available": r.is_ok(),
            "name": r.as_ref().ok().map(crate::llm::Endpoint::display_name),
            "model": r.as_ref().ok().map(|e| e.model.clone()),
            "endpoint": r.as_ref().ok().map(|e| e.base_url.clone()),
            "local": r.as_ref().ok().map(crate::llm::Endpoint::is_local),
            "problem": r.as_ref().err().map(ToString::to_string),
        })
    });
    let summary_name = summary.as_ref().ok().map(|b| b.display_name(settings));
    let ask_name = backend.as_ref().ok().map(Backend::display_name);
    // Said out loud when questions and summaries go different ways, which
    // only an older [meeting.summary] backend = "remote" does now.
    let summary_note = match (&ask_name, &summary_name) {
        (Some(a), Some(s)) if a != s => Some(format!(
            "Summaries use {s} ([meeting.summary] backend = \"{}\"); questions use {a}.",
            settings.summary_backend
        )),
        _ => None,
    };
    serde_json::json!({
        "agent": agent.map(|a| a.id.clone()),
        "agent_name": agent.map(|a| a.name.clone()),
        "agent_path": agent.map(|a| a.path.clone()),
        "source": res.source(),
        "consent": agent.is_some_and(|a| consent.is_allowed(&a.id)),
        "choice_required": res.choice_required,
        "problem": res.problem,
        "omarchy_default": res.omarchy_default,
        "local_only": settings.local_only,
        "local": {
            "model": settings.summary.ollama_model,
            "url": settings.summary.ollama_url,
            "name": format!("Local ({})", settings.summary.ollama_model),
        },
        "backend": backend_kind,
        "backend_name": ask_name,
        "blocked": blocked,
        "summary_backend": summary.as_ref().map(SummaryBackend::kind).unwrap_or("none"),
        "summary_backend_name": summary_name,
        "summary_blocked": summary.as_ref().err().map(AssistantError::code),
        "summary_note": summary_note,
        "auto_summary": settings.auto_summary(),
        "auto_summary_configured": settings.summary_auto,
        "consent_file": consent.path(),
        "candidates": res.candidates,
        "llm": llm_status,
    })
}

#[cfg(test)]
mod tests {
    use super::testing::Sandbox;
    use super::*;
    use crate::meeting::MeetingStorage;

    fn steve_meeting(sb: &Sandbox) -> MeetingData {
        // "Remote 2" on the third line stands in for a line moved there with
        // `meeting reassign`: storage hands the prompt builder segments with
        // overrides and labels already applied, as here.
        let m = sb.meeting(
            "Release sync",
            &[
                (41, "Remote 1", "We ship on Friday."),
                (52, "You", "Agreed."),
                (60, "Remote 2", "The budget is 40k."),
            ],
        );
        sb.label(&m, "You", "Pete");
        sb.label(&m, "Remote 1", "Steve");
        sb.label(&m, "Remote 2", "Dana");
        sb.reload(&m)
    }

    fn collect(
        settings: &AssistantSettings,
        meeting: &MeetingData,
        question: &str,
        last: Option<Duration>,
    ) -> (Result<asks::AskRecord, AssistantError>, String) {
        let backend = choose_backend(settings, &Overrides::default()).unwrap();
        let mut streamed = String::new();
        let r = ask(
            settings,
            meeting,
            &AskRequest {
                question: question.into(),
                last,
            },
            &backend,
            &mut |t| streamed.push_str(t),
        );
        (r, streamed)
    }

    #[test]
    fn without_consent_ask_refuses_and_names_the_command() {
        let sb = Sandbox::new();
        sb.fake_claude();
        let err = choose_backend(&sb.settings(), &Overrides::default()).unwrap_err();
        assert_eq!(err.code(), "consent_required");
        assert!(err
            .to_string()
            .contains("voxtype meeting assistant allow claude"));

        sb.allow("claude");
        match choose_backend(&sb.settings(), &Overrides::default()).unwrap() {
            Backend::Agent(a) => assert_eq!(a.id, "claude"),
            other => panic!("{other:?}"),
        }
        // Consent for one agent is not consent for another.
        sb.fake_codex();
        // Discovery must reach the consent gate even on a builder without
        // bwrap. This test never completes a Codex request or runs a jail.
        if !sb.has_bwrap() {
            crate::test_support::write_script(&sb.bin().join("bwrap"), "#!/bin/sh\nexit 97\n");
        }
        let err = choose_backend(
            &sb.settings(),
            &Overrides {
                agent: Some("codex".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, AssistantError::ConsentRequired { ref agent, .. } if agent == "codex")
        );
    }

    #[test]
    fn local_only_and_no_agent_use_the_local_model() {
        let sb = Sandbox::new();
        sb.fake_claude();
        let mut s = sb.settings();
        s.local_only = true;
        assert!(matches!(
            choose_backend(&s, &Overrides::default()).unwrap(),
            Backend::Local {
                fallback: false,
                ..
            }
        ));

        let empty = Sandbox::new();
        let b = choose_backend(&empty.settings(), &Overrides::default()).unwrap();
        assert!(matches!(b, Backend::Local { fallback: true, .. }));
        // Ollama is not running either: one message covering both.
        let err = b
            .complete("q", false, Duration::from_secs(2), &mut |_| {})
            .unwrap_err();
        assert_eq!(err.code(), "no_backend");
        assert!(err.to_string().contains("ollama pull llama3.2"), "{err}");
    }

    #[test]
    fn my_name_names_the_user_in_the_prompt() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.claude_replies(&["You agreed [00:52]."], "You agreed [00:52].");
        // No label on "You": storage opened without my_name, as a caller
        // that loads the meeting another way would hand it over.
        let m = sb.meeting(
            "Release sync",
            &[
                (41, "Remote 1", "We ship on Friday."),
                (52, "You", "Agreed."),
            ],
        );
        let m = sb.reload(&m);
        let mut settings = sb.settings();
        settings.my_name = Some("Pete".into());
        let (record, _) = collect(&settings, &m, "What did I agree to?", None);
        record.unwrap();
        let p = sb.claude_stdin(0);
        assert!(p.contains("Speakers: Remote 1, Pete (You)."), "{p}");
        assert!(p.contains("\nThe user asking is Pete (You).\n"), "{p}");
        assert!(p.contains("[00:52] Pete: Agreed.\n"), "{p}");
        assert!(!p.contains("] You:"), "{p}");
    }

    #[test]
    fn without_my_name_the_reader_is_the_you_speaker() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.claude_replies(&["ok"], "ok");
        let m = sb.meeting("Release sync", &[(52, "You", "Agreed.")]);
        let m = sb.reload(&m);
        collect(&sb.settings(), &m, "What did I say?", None)
            .0
            .unwrap();
        let p = sb.claude_stdin(0);
        assert!(p.contains("\nThe user asking is the speaker You.\n"), "{p}");
        assert!(p.contains("[00:52] You: Agreed.\n"), "{p}");
    }

    #[test]
    fn ask_sends_named_lines_and_records_the_answer() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.claude_replies(
            &["Steve said ", "we ship on Friday [00:41]."],
            "Steve said we ship on Friday [00:41].",
        );
        let m = steve_meeting(&sb);
        let storage = MeetingStorage::open(sb.storage_config()).unwrap();
        storage
            .set_notes(&m.metadata.id, "Ask about the budget\n")
            .unwrap();
        storage
            .annotate_segment(&m.metadata.id, 2, "check this number", true, true)
            .unwrap();
        let m = sb.reload(&m);

        let (record, streamed) = collect(&sb.settings(), &m, "What did Steve just say?", None);
        let record = record.unwrap();
        assert_eq!(streamed, "Steve said we ship on Friday [00:41].");
        assert_eq!(record.answer, "Steve said we ship on Friday [00:41].");
        assert_eq!(record.agent, "claude");
        assert_eq!(record.agent_name, "Claude");
        assert_eq!(
            record.citations,
            vec![prompt::Citation {
                ts: "00:41".into(),
                ms: 41_000
            }]
        );

        let p = sb.claude_stdin(0);
        assert!(p.starts_with("Meeting: Release sync, "), "{p}");
        assert!(
            p.contains("Speakers: Steve (Remote 1), Pete (You), Dana (Remote 2)."),
            "{p}"
        );
        assert!(p.contains("\nThe user asking is Pete (You).\n"), "{p}");
        assert!(p.contains("in the second person"), "{p}");
        assert!(p.contains(
            "[00:41] Steve: We ship on Friday.\n[00:52] Pete: Agreed.\n[01:00] Dana: The budget is 40k.\n"
        ));
        assert!(p.contains("User notes:\nAsk about the budget\n"), "{p}");
        assert!(
            p.contains("Annotations:\n[01:00] Highlight: check this number\n"),
            "{p}"
        );
        assert!(p.ends_with(
            "Question: What did Steve just say?\nAnswer briefly. Cite the [mm:ss] of the lines you rely on.\n"
        ));

        let argv = sb.claude_argv(0);
        assert_eq!(argv[0], "-p");
        let tools = argv.iter().position(|a| a == "--tools").unwrap();
        assert_eq!(argv[tools + 1], "");

        let saved = asks::load(sb.folder(&m));
        assert_eq!(saved, vec![record]);
    }

    #[test]
    fn last_limits_the_prompt_to_a_time_window() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.claude_replies(&[], "Late lines only [09:50].");
        let m = sb.meeting(
            "Long",
            &[
                (0, "You", "opening remarks"),
                (100, "Remote", "early topic"),
                (400, "Remote", "recent topic"),
                (590, "You", "closing"),
            ],
        );
        let storage = MeetingStorage::open(sb.storage_config()).unwrap();
        storage
            .annotate_segment(&m.metadata.id, 0, "early note", false, true)
            .unwrap();
        storage
            .annotate_segment(&m.metadata.id, 2, "late note", false, true)
            .unwrap();
        let m = sb.reload(&m);
        let (record, streamed) = collect(
            &sb.settings(),
            &m,
            "Summarize",
            Some(Duration::from_secs(300)),
        );
        let record = record.unwrap();
        // Nothing streamed as partials, so the result is shown whole.
        assert_eq!(streamed, "Late lines only [09:50].");
        assert_eq!(record.last_secs, Some(300));
        let p = sb.claude_stdin(0);
        assert!(
            p.contains(
                "Excerpt: only the last 5 minutes of the transcript, from [06:40] to [09:53]."
            ),
            "{p}"
        );
        assert!(p.contains("recent topic") && p.contains("closing"));
        assert!(p.contains("Annotations:\n[06:40] Note: late note\n"), "{p}");
        assert!(!p.contains("early note"));
        assert!(!p.contains("opening remarks") && !p.contains("early topic"));
    }

    #[test]
    fn a_long_meeting_sends_a_running_summary_plus_recent_lines() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.claude_replies(&[], "They covered topics one to thirty.");
        let lines: Vec<(u64, String)> = (0..60)
            .map(|i| (i * 10, format!("topic number {i} discussed at some length")))
            .collect();
        let refs: Vec<(u64, &str, &str)> = lines
            .iter()
            .map(|(s, t)| (*s, if s % 20 == 0 { "You" } else { "Remote" }, t.as_str()))
            .collect();
        let m = sb.meeting("Marathon", &refs);
        let mut s = sb.settings();
        s.max_transcript_chars = 1_200;

        let (r, _) = collect(&s, &m, "What was decided?", None);
        r.unwrap();
        let calls = sb.claude_calls();
        assert!(calls >= 2, "earlier part summarized first ({calls} calls)");
        let p = sb.claude_stdin(calls - 1);
        assert!(p.contains("Summary of the meeting up to ["), "{p}");
        assert!(p.contains("They covered topics one to thirty."));
        assert!(!p.contains("topic number 0 "));
        assert!(p.contains("topic number 59 "));
        assert!(
            p.len() < 1_200 + 800,
            "prompt stays near the budget: {}",
            p.len()
        );
        assert!(sb.folder(&m).join(CONTEXT_CACHE_FILE).exists());

        // The next question reuses the cached summary: one more call only.
        let (r, _) = collect(&s, &m, "And then?", None);
        r.unwrap();
        assert_eq!(sb.claude_calls(), calls + 1);
    }

    #[test]
    fn codex_runs_in_a_jail_that_hides_home() {
        let sb = Sandbox::new();
        if !sb.has_bwrap() {
            eprintln!("skipping: bwrap is not installed");
            return;
        }
        sb.fake_codex();
        sb.allow("codex");
        let m = steve_meeting(&sb);
        let mut s = sb.settings();
        s.agent = Some("codex".into());
        let (record, streamed) = collect(&s, &m, "When do we ship?", None);
        let report = record.unwrap().answer;
        assert_eq!(streamed, report);

        // Nothing from the real home, the meetings or the caller's files.
        assert!(report.contains("SENTINEL_HIDDEN"), "{report}");
        assert!(!report.contains("SENTINEL-KEY"), "{report}");
        assert!(report.contains("MEETINGS_HIDDEN"), "{report}");
        assert!(report.contains("OUT_HIDDEN"), "{report}");
        // Its own auth, read-only, in a fresh home and an empty cwd.
        assert!(report.contains(r#"AUTH={"fake":"auth"}"#), "{report}");
        assert!(report.contains("AUTH_READONLY"), "{report}");
        assert!(
            report.contains(
                "HOME=/home/voxtype-jail CODEX_HOME=/home/voxtype-jail/.codex PWD=/tmp/work"
            ),
            "{report}"
        );
        assert!(report.contains("CWD_LIST=[]"), "{report}");
        // The flags still apply inside the jail, and the prompt arrives.
        assert!(
            report.contains("ARGV: exec --sandbox read-only --skip-git-repo-check"),
            "{report}"
        );
        assert!(
            report.contains("-C /tmp/work -o /tmp/out/last-message.txt -\n"),
            "{report}"
        );
        assert!(report.contains("Question: When do we ship?"), "{report}");
        // The real auth.json is untouched.
        assert_eq!(
            std::fs::read_to_string(sb.home().join(".codex/auth.json")).unwrap(),
            r#"{"fake":"auth"}"#
        );
    }

    #[test]
    fn pi_answers_with_its_tools_off_from_an_attached_prompt() {
        let sb = Sandbox::new();
        sb.fake_pi();
        sb.allow("pi");
        std::fs::write(sb.out().join("pi.reply"), "Pi says Friday [00:41].\n").unwrap();
        let m = steve_meeting(&sb);
        let mut s = sb.settings();
        s.agent = Some("pi".into());
        let (record, streamed) = collect(&s, &m, "When do we ship?", None);
        let record = record.unwrap();
        assert_eq!(record.answer, "Pi says Friday [00:41].");
        assert_eq!(record.agent, "pi");
        assert_eq!(record.agent_name, "Pi");
        assert_eq!(streamed, "Pi says Friday [00:41].\n");
        let argv = std::fs::read_to_string(sb.out().join("pi.argv")).unwrap();
        let argv: Vec<&str> = argv.lines().collect();
        assert_eq!(argv[0], "--print");
        assert!(argv.contains(&"--no-tools") && argv.contains(&"--no-mcp"));
        assert!(argv.contains(&"--no-extensions") && argv.contains(&"--no-context-files"));
        assert_eq!(*argv.last().unwrap(), runner::PI_INSTRUCTION);
        let prompt = std::fs::read_to_string(sb.out().join("pi.prompt")).unwrap();
        assert!(
            prompt.contains("[00:41] Steve: We ship on Friday."),
            "{prompt}"
        );
        assert!(prompt.contains("Question: When do we ship?"), "{prompt}");
    }

    #[test]
    fn codex_is_refused_without_bwrap() {
        let sb = Sandbox::new();
        sb.fake_codex();
        sb.allow("codex");
        let _ = std::fs::remove_file(sb.bin().join("bwrap"));
        let mut s = sb.settings();
        s.agent = Some("codex".into());
        let err = choose_backend(&s, &Overrides::default()).unwrap_err();
        assert_eq!(err.code(), "agent_unavailable");
        assert!(err.to_string().contains("bubblewrap"), "{err}");
    }

    #[test]
    fn agent_failures_and_timeouts_are_reported() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        sb.claude_replies(&[], "unused");
        let m = steve_meeting(&sb);
        std::fs::write(sb.out().join("claude.fail"), "").unwrap();
        let (r, _) = collect(&sb.settings(), &m, "q", None);
        let err = r.unwrap_err();
        assert_eq!(err.code(), "agent_failed");
        assert!(err.to_string().contains("Not logged in"), "{err}");
        assert!(
            asks::load(sb.folder(&m)).is_empty(),
            "failed asks are not saved"
        );

        std::fs::remove_file(sb.out().join("claude.fail")).unwrap();
        std::fs::write(sb.out().join("claude.slow"), "").unwrap();
        let mut s = sb.settings();
        s.timeout = Duration::from_secs(1);
        let started = std::time::Instant::now();
        let (r, _) = collect(&s, &m, "q", None);
        assert_eq!(r.unwrap_err().code(), "timeout");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn status_reports_agent_candidates_and_consent() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.fake_codex();
        sb.fake_omarchy("claude");
        let v = status_json(&sb.settings());
        assert_eq!(v["agent"], "claude");
        assert_eq!(v["agent_name"], "Claude");
        assert_eq!(v["source"], "omarchy");
        assert_eq!(v["omarchy_default"], "claude");
        assert_eq!(v["consent"], false);
        assert_eq!(v["backend"], "none");
        assert_eq!(v["blocked"], "consent_required");
        assert_eq!(v["auto_summary"], false);
        assert_eq!(v["local_only"], false);
        assert_eq!(v["local"]["name"], "Local (llama3.2)");
        let c = v["candidates"].as_array().unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0]["id"], "claude");
        assert_eq!(c[0]["tool_less"], true);
        assert_eq!(c[1]["id"], "codex");
        assert_eq!(c[1]["tool_less"], false);
        assert_eq!(c[1]["sandbox"], "bwrap");
        assert_eq!(c[0]["sandbox"], serde_json::Value::Null);

        sb.allow("claude");
        let v = status_json(&sb.settings());
        assert_eq!(v["consent"], true);
        assert_eq!(v["backend"], "agent");
        assert_eq!(v["backend_name"], "Claude");
        assert_eq!(v["auto_summary"], true);
        assert_eq!(v["candidates"][0]["consent"], true);
    }

    #[test]
    fn open_defaults_to_the_meeting_view_of_this_binary() {
        let exe = Path::new("/opt/vox type/voxtype");
        assert_eq!(
            open_action_command(None, Some(exe), |p| p == exe).as_deref(),
            Some("'/opt/vox type/voxtype' meeting view \"$VOXTYPE_MEETING_ID\" --tab ask")
        );
        // A binary without `meeting view` gets no Open action.
        assert_eq!(open_action_command(None, Some(exe), |_| false), None);
        assert_eq!(open_action_command(None, None, |_| true), None);
        // Explicit settings win; empty disables.
        assert_eq!(
            open_action_command(Some("my-view --ask"), Some(exe), |_| true).as_deref(),
            Some("my-view --ask")
        );
        assert_eq!(open_action_command(Some(""), Some(exe), |_| true), None);
        assert_eq!(open_action_command(Some("  "), Some(exe), |_| true), None);
        assert_eq!(shell_quote("/usr/bin/voxtype"), "/usr/bin/voxtype");
        assert_eq!(shell_quote("/a'b"), "'/a'\\''b'");
    }

    #[test]
    fn durations_parse() {
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration("2m30s").unwrap(), Duration::from_secs(150));
        assert_eq!(parse_duration("10").unwrap(), Duration::from_secs(600));
        assert!(parse_duration("5x").is_err());
        assert!(parse_duration("m").is_err());
        assert!(parse_duration("5m3").is_err());
        assert_eq!(human_duration(Duration::from_secs(300)), "5 minutes");
        assert_eq!(human_duration(Duration::from_secs(60)), "1 minute");
        assert_eq!(human_duration(Duration::from_secs(90)), "90 seconds");
        assert_eq!(human_duration(Duration::from_secs(5400)), "1h 30m");
    }

    // -- Omarchy's local model ----------------------------------------------

    use crate::llm::fake_server::{FakeServer, Reply};

    /// Ask counts Omarchy's local model as local: no agent-allow prompt,
    /// even with an agent on PATH that has no consent, and the agent is
    /// never run.
    #[test]
    fn omarchy_answers_without_agent_consent() {
        let sb = Sandbox::new();
        sb.fake_claude();
        let m = steve_meeting(&sb);
        let server = FakeServer::start(vec![Reply::stream(&["Steve said ", "Friday [00:41]."])]);
        let mut s = sb.settings();
        sb.use_omarchy(&mut s, "active", &server.base_url());

        let backend = choose_backend(&s, &Overrides::default()).unwrap();
        assert!(matches!(&backend, Backend::Llm(e) if e.model == "qwen-test" && e.is_local()));
        assert_eq!(backend.display_name(), "Omarchy (qwen-test)");

        let (r, streamed) = collect(&s, &m, "When do we ship?", None);
        let record = r.unwrap();
        assert_eq!(record.answer, "Steve said Friday [00:41].");
        assert_eq!(streamed, "Steve said Friday [00:41].");
        assert_eq!(record.agent, "omarchy:qwen-test");
        assert_eq!(sb.claude_calls(), 0);
        let req = &server.requests()[0];
        assert_eq!(req.authorization.as_deref(), Some("Bearer fixture-key"));
        assert!(req.body.contains("We ship on Friday."));

        let v = status_json(&s);
        assert_eq!(v["backend"], "local");
        assert_eq!(v["blocked"], serde_json::Value::Null);
        assert_eq!(v["llm"]["available"], true);
        assert_eq!(v["llm"]["model"], "qwen-test");
    }

    /// `local_only` keeps transcripts off agents; Omarchy's model is local,
    /// so it answers rather than Ollama.
    #[test]
    fn local_only_prefers_omarchy_over_ollama() {
        let sb = Sandbox::new();
        let server = FakeServer::start(vec![]);
        let mut s = sb.settings();
        s.local_only = true;
        sb.use_omarchy(&mut s, "active", &server.base_url());
        assert!(matches!(
            choose_backend(&s, &Overrides::default()).unwrap(),
            Backend::Llm(_)
        ));
    }

    /// With no Omarchy model running, Ask decides as if Omarchy were not
    /// chosen: an allowed agent answers, an unallowed one still needs
    /// consent, and with none at all the Ollama fallback is used.
    #[test]
    fn no_omarchy_model_running_falls_back_to_the_usual_order() {
        let sb = Sandbox::new();
        let mut s = sb.settings();
        sb.use_omarchy(&mut s, "inactive", "http://127.0.0.1:9/v1");
        assert!(matches!(
            choose_backend(&s, &Overrides::default()).unwrap(),
            Backend::Local { fallback: true, .. }
        ));
        let v = status_json(&s);
        assert_eq!(v["llm"]["available"], false);
        assert!(v["llm"]["problem"]
            .as_str()
            .unwrap()
            .contains("omarchy models start qwen-test"));

        sb.fake_claude();
        let err = choose_backend(&s, &Overrides::default()).unwrap_err();
        assert_eq!(err.code(), "consent_required");
        sb.allow("claude");
        assert!(matches!(
            choose_backend(&s, &Overrides::default()).unwrap(),
            Backend::Agent(_)
        ));
    }

    /// `--agent` is an explicit choice for one call and wins over Omarchy.
    #[test]
    fn an_explicit_agent_wins_over_omarchy() {
        let sb = Sandbox::new();
        sb.fake_claude();
        sb.allow("claude");
        let server = FakeServer::start(vec![]);
        let mut s = sb.settings();
        sb.use_omarchy(&mut s, "active", &server.base_url());
        let b = choose_backend(
            &s,
            &Overrides {
                agent: Some("claude".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(b, Backend::Agent(a) if a.id == "claude"));
        assert!(server.requests().is_empty());
    }

    /// A remote "openai" server writes summaries when chosen, but never
    /// answers asks and never serves `local_only`.
    #[test]
    fn a_remote_openai_server_is_for_summaries_only() {
        let sb = Sandbox::new();
        let mut s = sb.settings();
        s.summary_backend = "openai".into();
        s.llm = Some(crate::llm::LlmConfig {
            provider: crate::llm::Provider::OpenAi,
            url: Some("https://llm.example.com/v1".into()),
            model: Some("big".into()),
            api_key_file: None,
            settings: crate::llm::SettingNames {
                table: "meeting.summary",
                url: "openai_url",
                model: "openai_model",
            },
        });
        assert!(matches!(
            choose_backend(&s, &Overrides::default()).unwrap(),
            Backend::Local { .. }
        ));
        match choose_summary_backend(&s, &Overrides::default()).unwrap() {
            SummaryBackend::Assistant(Backend::Llm(e)) => assert!(!e.is_local()),
            other => panic!("{other:?}"),
        }
        s.local_only = true;
        assert!(matches!(
            choose_summary_backend(&s, &Overrides::default()).unwrap(),
            SummaryBackend::Assistant(Backend::Local { .. })
        ));
    }
}
