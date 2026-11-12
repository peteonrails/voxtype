//! What a meeting view does when a button is pressed: run the matching
//! `voxtype meeting` command, so it gets the same checks and messages as
//! the command line. Toolkit-free; the terminal view and the GTK window
//! both use it.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// How to run `voxtype`: the binary and arguments that go before the
/// subcommand (`--config <path>`).
#[derive(Debug, Clone)]
pub struct Cli {
    pub exe: PathBuf,
    pub global_args: Vec<String>,
}

impl Cli {
    /// The running binary, passing on the `--config` it was started with.
    pub fn current() -> std::io::Result<Self> {
        Ok(Self {
            exe: std::env::current_exe()?,
            global_args: crate::config::Config::path_override()
                .map(|p| vec!["--config".to_string(), p.display().to_string()])
                .unwrap_or_default(),
        })
    }

    fn command(&self, args: &[String]) -> Command {
        let mut cmd = Command::new(&self.exe);
        cmd.args(&self.global_args).args(args);
        cmd
    }

    /// Run `args` to completion, feeding `stdin` if given.
    pub fn run(&self, request: &Request) -> Outcome {
        let mut cmd = self.command(&request.args);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd.stdin(if request.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        let result = cmd.spawn().and_then(|mut child| {
            if let (Some(text), Some(mut stdin)) = (&request.stdin, child.stdin.take()) {
                let _ = stdin.write_all(text.as_bytes());
            }
            child.wait_with_output()
        });
        match result {
            Ok(o) => {
                let ok = o.status.success();
                let text = String::from_utf8_lossy(if ok { &o.stdout } else { &o.stderr });
                Outcome {
                    what: request.what.clone(),
                    ok,
                    message: first_line(&text),
                    reload: request.reload,
                }
            }
            Err(e) => Outcome {
                what: request.what.clone(),
                ok: false,
                message: e.to_string(),
                reload: false,
            },
        }
    }

    /// Run `meeting ask --json`, calling `on_line` for each JSON line.
    /// Returns whether it exited successfully.
    pub fn ask(&self, meeting_id: &str, question: &str, mut on_line: impl FnMut(&str)) -> bool {
        let args = ask(meeting_id, question);
        let child = self
            .command(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else {
            return false;
        };
        if let Some(out) = child.stdout.take() {
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                on_line(&line);
            }
        }
        child.wait().map(|s| s.success()).unwrap_or(false)
    }
}

/// The first non-empty line of a command's output, without an "Error: "
/// prefix, for a status line.
fn first_line(text: &str) -> String {
    text.lines()
        .map(|l| l.trim().trim_start_matches("Error: ").trim())
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// A command for the view to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// What to call it in a status line: "Pause", "Reassign", ...
    pub what: String,
    pub args: Vec<String>,
    /// Text for the command's stdin.
    pub stdin: Option<String>,
    /// Read the meeting again afterwards (it has ended, so the socket will
    /// not say what changed).
    pub reload: bool,
}

/// How a command went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub what: String,
    pub ok: bool,
    pub message: String,
    pub reload: bool,
}

impl Outcome {
    /// One line for a status bar.
    pub fn status_line(&self) -> String {
        if self.ok {
            format!("{}: done", self.what)
        } else {
            format!("{} failed: {}", self.what, self.message)
        }
    }
}

fn args<const N: usize>(a: [&str; N]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

pub fn pause() -> Request {
    Request {
        what: "Pause".into(),
        args: args(["meeting", "pause"]),
        stdin: None,
        reload: false,
    }
}

pub fn resume() -> Request {
    Request {
        what: "Resume".into(),
        args: args(["meeting", "resume"]),
        stdin: None,
        reload: false,
    }
}

/// Start a meeting, titled `title` when it is not empty.
pub fn start(title: &str) -> Request {
    let mut args = args(["meeting", "start"]);
    if !title.trim().is_empty() {
        args.extend(["--title".to_string(), title.trim().to_string()]);
    }
    Request {
        what: "Start".into(),
        args,
        stdin: None,
        reload: false,
    }
}

pub fn stop() -> Request {
    Request {
        what: "Stop".into(),
        args: args(["meeting", "stop"]),
        stdin: None,
        reload: false,
    }
}

/// Move segment `segment` to `speaker` (an ID, a name, or a new name).
pub fn reassign(meeting_id: &str, segment: u32, speaker: &str) -> Request {
    Request {
        what: "Reassign".into(),
        args: args([
            "meeting",
            "reassign",
            meeting_id,
            &segment.to_string(),
            speaker,
        ]),
        stdin: None,
        reload: true,
    }
}

/// Note on segment `segment`.
pub fn annotate(meeting_id: &str, segment: u32, text: &str) -> Request {
    Request {
        what: "Note".into(),
        args: args([
            "meeting",
            "annotate",
            meeting_id,
            &segment.to_string(),
            text,
        ]),
        stdin: None,
        reload: true,
    }
}

/// Name speaker `speaker` (an ID or a name it already has) `name`.
pub fn label(meeting_id: &str, speaker: &str, name: &str) -> Request {
    Request {
        what: "Rename".into(),
        args: args(["meeting", "label", meeting_id, speaker, name]),
        stdin: None,
        reload: true,
    }
}

/// Replace the meeting's notes with `text`.
pub fn set_notes(meeting_id: &str, text: &str) -> Request {
    Request {
        what: "Notes".into(),
        args: args(["meeting", "notes", meeting_id, "--set-file", "-"]),
        stdin: Some(text.to_string()),
        reload: false,
    }
}

/// Arguments for `meeting ask --json`.
pub fn ask(meeting_id: &str, question: &str) -> Vec<String> {
    args(["meeting", "ask", meeting_id, question, "--json"])
}

/// An answer streaming in from `meeting ask --json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskAnswer {
    pub question: String,
    pub answer: String,
    pub agent_name: Option<String>,
    pub done: bool,
    pub error: Option<String>,
}

impl AskAnswer {
    pub fn new(question: &str) -> Self {
        Self {
            question: question.to_string(),
            ..Default::default()
        }
    }

    /// Take one `meeting ask --json` line.
    pub fn apply_line(&mut self, line: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            return;
        };
        match v["event"].as_str() {
            Some("start") => self.agent_name = v["agent_name"].as_str().map(String::from),
            Some("delta") => self.answer.push_str(v["text"].as_str().unwrap_or_default()),
            Some("answer") => {
                if let Some(a) = v["answer"].as_str() {
                    self.answer = a.to_string();
                }
                if let Some(n) = v["agent_name"].as_str() {
                    self.agent_name = Some(n.to_string());
                }
                self.done = true;
            }
            Some("error") => {
                self.error = Some(v["message"].as_str().unwrap_or("failed").to_string());
                self.done = true;
            }
            _ => {}
        }
    }

    /// The command exited.
    pub fn finished(&mut self, ok: bool) {
        if !self.done {
            self.done = true;
            if !ok && self.error.is_none() {
                self.error = Some("The assistant did not answer".into());
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn requests_name_the_meeting_commands() {
        assert_eq!(
            reassign("m1", 4, "Dana").args,
            ["meeting", "reassign", "m1", "4", "Dana"]
        );
        assert_eq!(
            annotate("m1", 4, "check").args,
            ["meeting", "annotate", "m1", "4", "check"]
        );
        assert_eq!(
            label("m1", "Remote 1", "Steve").args,
            ["meeting", "label", "m1", "Remote 1", "Steve"]
        );
        let notes = set_notes("m1", "agenda");
        assert_eq!(notes.args, ["meeting", "notes", "m1", "--set-file", "-"]);
        assert_eq!(notes.stdin.as_deref(), Some("agenda"));
        assert_eq!(ask("m1", "q"), ["meeting", "ask", "m1", "q", "--json"]);
        assert!(stop().args.ends_with(&["stop".to_string()]));
    }

    #[test]
    fn answers_stream_and_errors_show() {
        let mut a = AskAnswer::new("What did Steve say?");
        a.apply_line(r#"{"event":"start","agent_name":"Claude"}"#);
        a.apply_line(r#"{"event":"delta","text":"Steve "}"#);
        a.apply_line(r#"{"event":"delta","text":"said hi"}"#);
        assert_eq!((a.answer.as_str(), a.done), ("Steve said hi", false));
        a.apply_line(r#"{"event":"answer","answer":"Steve said hi [00:41]."}"#);
        assert!(a.done);
        assert_eq!(a.answer, "Steve said hi [00:41].");
        assert_eq!(a.agent_name.as_deref(), Some("Claude"));

        let mut b = AskAnswer::new("q");
        b.apply_line(r#"{"event":"error","code":"consent_required","message":"Allow it"}"#);
        assert_eq!(b.error.as_deref(), Some("Allow it"));
        let mut c = AskAnswer::new("q");
        c.finished(false);
        assert!(c.done && c.error.is_some());
    }

    #[test]
    fn commands_run_with_stdin_and_report_errors() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("fake-voxtype");
        crate::test_support::write_script(
            &exe,
            "#!/bin/sh\nif [ \"$6\" = fail ]; then echo 'Error: no such meeting' >&2; exit 1; fi\ncat > \"$(dirname \"$0\")/stdin\"\necho \"$@\"\n",
        );
        let cli = Cli {
            exe,
            global_args: vec!["--config".into(), "/c.toml".into()],
        };
        let out = cli.run(&set_notes("m1", "agenda"));
        assert!(out.ok, "{out:?}");
        assert_eq!(
            out.message,
            "--config /c.toml meeting notes m1 --set-file -"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("stdin")).unwrap(),
            "agenda"
        );
        let out = cli.run(&label("m1", "fail", "x"));
        assert!(!out.ok);
        assert_eq!(out.status_line(), "Rename failed: no such meeting");
    }

    #[test]
    fn start_passes_a_title_only_when_given() {
        assert_eq!(start("  ").args, vec!["meeting", "start"]);
        assert_eq!(
            start(" Standup ").args,
            vec!["meeting", "start", "--title", "Standup"]
        );
    }
}
