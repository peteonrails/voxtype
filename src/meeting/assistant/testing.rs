//! Test fixtures: fake agent CLIs on a private PATH and a stored meeting.
//! Nothing here runs a real agent.

use super::{AssistantEnv, AssistantSettings};
use crate::meeting::data::{
    AudioSource, MeetingData, MeetingMetadata, Transcript, TranscriptSegment,
};
use crate::meeting::storage::{MeetingStorage, StorageConfig};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A temp dir with `bin/` (fake agents), `out/` (what they saw) and
/// `meetings/` (storage).
pub struct Sandbox {
    pub dir: tempfile::TempDir,
}

/// One line of claude's stream-json carrying `text` as a partial delta.
pub fn claude_delta(text: &str) -> String {
    serde_json::json!({
        "type": "stream_event",
        "event": {"type": "content_block_delta", "index": 0,
                  "delta": {"type": "text_delta", "text": text}}
    })
    .to_string()
}

/// claude's final `result` line.
pub fn claude_result(text: &str) -> String {
    serde_json::json!({"type": "result", "subtype": "success", "is_error": false, "result": text})
        .to_string()
}

impl Sandbox {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for d in ["bin", "out", "meetings", "home/.ssh", "home/.codex"] {
            std::fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        // What the codex jail must hide: a key under the real HOME. And
        // what it must let through: codex's auth.json.
        std::fs::write(dir.path().join("home/.ssh/id_sentinel"), "SENTINEL-KEY\n").unwrap();
        std::fs::write(
            dir.path().join("home/.codex/auth.json"),
            "{\"fake\":\"auth\"}",
        )
        .unwrap();
        if let Ok(bwrap) = which::which("bwrap") {
            std::os::unix::fs::symlink(bwrap, dir.path().join("bin/bwrap")).unwrap();
        }
        Self { dir }
    }

    pub fn bin(&self) -> PathBuf {
        self.dir.path().join("bin")
    }

    pub fn out(&self) -> PathBuf {
        self.dir.path().join("out")
    }

    fn script(&self, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let p = self.bin().join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A fake `claude` that records argv and stdin (one file per call,
    /// numbered) and prints `out/claude.reply`, or fails when
    /// `out/claude.fail` exists.
    pub fn fake_claude(&self) {
        let out = self.out();
        self.script(
            "claude",
            &format!(
                r#"out='{out}'
n=$(ls "$out" | grep -c '^claude\.stdin\.' )
printf '%s\n' "$@" > "$out/claude.argv.$n"
cat > "$out/claude.stdin.$n"
if [ -f "$out/claude.slow" ]; then sleep 10; fi
if [ -f "$out/claude.fail" ]; then echo 'Not logged in. Run claude /login' >&2; exit 3; fi
cat "$out/claude.reply""#,
                out = out.display()
            ),
        );
    }

    /// Set what the fake claude streams: deltas then the final result.
    pub fn claude_replies(&self, deltas: &[&str], result: &str) {
        let mut lines: Vec<String> = vec![r#"{"type":"system","subtype":"init"}"#.to_string()];
        lines.extend(deltas.iter().map(|d| claude_delta(d)));
        lines.push(claude_result(result));
        std::fs::write(self.out().join("claude.reply"), lines.join("\n") + "\n").unwrap();
    }

    /// The fake home that stands in for the real HOME.
    pub fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    /// Is bwrap available here? The codex tests need it.
    pub fn has_bwrap(&self) -> bool {
        self.bin().join("bwrap").exists()
    }

    /// A fake `codex` that, from inside the jail, reports what it can see:
    /// its argv, HOME, cwd, whether the HOME sentinel, the meetings folder,
    /// the sandbox's output folder and auth.json are readable, whether
    /// auth.json is writable, and its stdin. The report is its answer, so
    /// it comes back out through the jail's one writable mount.
    pub fn fake_codex(&self) {
        let sentinel = self.home().join(".ssh/id_sentinel");
        self.script(
            "codex",
            &format!(
                r#"last=''; prev=''
for a in "$@"; do [ "$prev" = -o ] && last="$a"; prev="$a"; done
{{
printf 'ARGV:'; printf ' %s' "$@"; echo
echo "HOME=$HOME CODEX_HOME=$CODEX_HOME PWD=$(pwd)"
echo "CWD_LIST=[$(ls -A)]"
if cat '{sentinel}' 2>/dev/null; then echo SENTINEL_VISIBLE; else echo SENTINEL_HIDDEN; fi
if ls '{meetings}' >/dev/null 2>&1; then echo MEETINGS_VISIBLE; else echo MEETINGS_HIDDEN; fi
if ls '{out}' >/dev/null 2>&1; then echo OUT_VISIBLE; else echo OUT_HIDDEN; fi
echo "AUTH=$(cat "$CODEX_HOME/auth.json" 2>/dev/null)"
if (echo x > "$CODEX_HOME/auth.json") 2>/dev/null; then echo AUTH_WRITABLE; else echo AUTH_READONLY; fi
echo 'STDIN<<'; cat; echo '>>'
}} > "$last""#,
                sentinel = sentinel.display(),
                meetings = self.dir.path().join("meetings").display(),
                out = self.out().display(),
            ),
        );
    }

    pub fn fake_omarchy(&self, prints: &str) {
        self.script(
            "omarchy-default-agent",
            &format!("[ $# -eq 0 ] || exit 9\necho '{prints}'"),
        );
    }

    /// The nth prompt the fake claude was given.
    pub fn claude_stdin(&self, n: usize) -> String {
        std::fs::read_to_string(self.out().join(format!("claude.stdin.{n}"))).unwrap()
    }

    pub fn claude_argv(&self, n: usize) -> Vec<String> {
        std::fs::read_to_string(self.out().join(format!("claude.argv.{n}")))
            .unwrap()
            .lines()
            .map(String::from)
            .collect()
    }

    pub fn claude_calls(&self) -> usize {
        std::fs::read_dir(self.out())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("claude.stdin.")
            })
            .count()
    }

    pub fn storage_config(&self) -> StorageConfig {
        StorageConfig {
            storage_path: self.dir.path().join("meetings"),
            retain_audio: false,
            max_meetings: 0,
        }
    }

    /// Settings that find agents only in `bin/`, keep consent in the
    /// sandbox, and point the local model at a closed port.
    pub fn settings(&self) -> AssistantSettings {
        let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", dead.local_addr().unwrap());
        drop(dead);
        AssistantSettings {
            agent: None,
            local_only: false,
            timeout: Duration::from_secs(20),
            max_transcript_chars: 100_000,
            open_command: None,
            summary_auto: None,
            summary_backend: "disabled".into(),
            summary: crate::meeting::summary::SummaryConfig {
                ollama_url: url,
                ..Default::default()
            },
            env: AssistantEnv {
                path: Some(OsString::from(self.bin())),
                consent_file: self.dir.path().join("state").join("assistant-consent.json"),
                omarchy_command: "omarchy-default-agent".into(),
                codex_home: self.home().join(".codex"),
            },
        }
    }

    pub fn allow(&self, agent: &str) {
        let s = self.settings();
        s.consent()
            .allow(agent, Some(self.bin().join(agent)))
            .unwrap();
    }

    /// Store a finished meeting with `lines` of (seconds, speaker id, text).
    pub fn meeting(&self, title: &str, lines: &[(u64, &str, &str)]) -> MeetingData {
        let storage = MeetingStorage::open(self.storage_config()).unwrap();
        let mut metadata = MeetingMetadata::new(Some(title.to_string()));
        metadata.storage_path = Some(storage.create_meeting(&metadata).unwrap());
        let mut transcript = Transcript::new();
        for (i, (secs, speaker, text)) in lines.iter().enumerate() {
            let mut seg = TranscriptSegment::new(
                i as u32,
                secs * 1000,
                secs * 1000 + 3000,
                text.to_string(),
                0,
            );
            seg.source = if *speaker == "You" {
                AudioSource::Microphone
            } else {
                AudioSource::Loopback
            };
            seg.speaker_id = Some(speaker.to_string());
            transcript.add_segment(seg);
        }
        transcript.total_chunks = 1;
        storage.save_transcript(&metadata.id, &transcript).unwrap();
        metadata.complete();
        metadata.duration_secs = Some(lines.last().map(|l| l.0 + 3).unwrap_or(0));
        metadata.chunk_count = 1;
        storage.update_meeting(&metadata).unwrap();
        storage.load_meeting_data(&metadata.id).unwrap()
    }

    pub fn reload(&self, meeting: &MeetingData) -> MeetingData {
        MeetingStorage::open(self.storage_config())
            .unwrap()
            .load_meeting_data(&meeting.metadata.id)
            .unwrap()
    }

    pub fn label(&self, meeting: &MeetingData, speaker: &str, name: &str) {
        MeetingStorage::open(self.storage_config())
            .unwrap()
            .set_speaker_label(&meeting.metadata.id, speaker, name)
            .unwrap();
    }

    pub fn folder<'a>(&self, meeting: &'a MeetingData) -> &'a Path {
        meeting.metadata.storage_path.as_deref().unwrap()
    }
}
