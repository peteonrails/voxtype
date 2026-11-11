//! `voxtype setup skill`: install the `voxtype-setup` Agent Skill for the
//! coding agents on this machine.
//!
//! The skill is compiled into the binary, so every install method (packages,
//! AUR, Nix, AppImage, cargo) can install it, and the copy it writes always
//! matches the running version. After an upgrade the installed copy can be
//! older than the embedded one; `--status` and `voxtype setup check` say so.
//!
//! Package scripts run as root and must not write into home directories, so
//! the user runs this command themselves.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

/// Directory name agents load the skill from.
pub const SKILL_NAME: &str = "voxtype-setup";

/// Beside SKILL.md, the SHA-256 of the copy voxtype last wrote. A SKILL.md that
/// no longer matches it was edited by the user, so it is never replaced or
/// removed without asking.
const MARKER: &str = ".voxtype-installed";

/// The skill shipped with this version of voxtype.
pub const SKILL_MD: &str = include_str!("../../skills/voxtype-setup/SKILL.md");

/// Where a group of agents loads personal skills from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Claude Code reads `~/.claude/skills`.
    Claude,
    /// The cross-agent convention, `~/.agents/skills`, read by Codex, Gemini
    /// CLI, OpenCode, Cursor, GitHub Copilot and Goose.
    Agents,
}

impl Target {
    pub const ALL: [Target; 2] = [Target::Claude, Target::Agents];

    pub fn id(self) -> &'static str {
        match self {
            Target::Claude => "claude",
            Target::Agents => "agents",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Target::Claude => "Claude Code",
            Target::Agents => "Codex, Gemini CLI, OpenCode, Cursor, Copilot, Goose",
        }
    }

    /// The skill's directory under `home`.
    pub fn dir(self, home: &Path) -> PathBuf {
        let skills = match self {
            Target::Claude => home.join(".claude/skills"),
            Target::Agents => home.join(".agents/skills"),
        };
        skills.join(SKILL_NAME)
    }

    /// Config directories under `home` whose presence means the agent is used.
    fn config_dirs(self) -> &'static [&'static str] {
        match self {
            Target::Claude => &[".claude"],
            Target::Agents => &[
                ".agents",
                ".codex",
                ".gemini",
                ".config/opencode",
                ".cursor",
                ".copilot",
                ".config/goose",
            ],
        }
    }

    /// Executables whose presence on PATH means the agent is installed.
    fn binaries(self) -> &'static [&'static str] {
        match self {
            Target::Claude => &["claude"],
            Target::Agents => &[
                "codex",
                "gemini",
                "opencode",
                "cursor-agent",
                "copilot",
                "goose",
            ],
        }
    }

    pub fn detected(self, home: &Path, on_path: &dyn Fn(&str) -> bool) -> bool {
        self.config_dirs().iter().any(|d| home.join(d).is_dir())
            || self.binaries().iter().any(|b| on_path(b))
    }
}

/// Parse a `--target` value: `claude`, `agents` or `all`.
pub fn parse_targets(value: &str) -> Option<Vec<Target>> {
    match value {
        "claude" => Some(vec![Target::Claude]),
        "agents" => Some(vec![Target::Agents]),
        "all" => Some(Target::ALL.to_vec()),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    NotInstalled,
    /// Identical to the skill embedded in this binary.
    Current,
    /// An unedited copy an older voxtype wrote.
    Outdated,
    /// A voxtype-setup skill the user has edited since voxtype wrote it.
    Modified,
    /// A different skill sits at this path.
    Foreign,
}

impl State {
    pub fn id(self) -> &'static str {
        match self {
            State::NotInstalled => "not_installed",
            State::Current => "current",
            State::Outdated => "outdated",
            State::Modified => "modified",
            State::Foreign => "foreign",
        }
    }

    /// Whether replacing or removing the file would lose the user's work.
    pub fn needs_consent(self) -> bool {
        matches!(self, State::Modified | State::Foreign)
    }
}

fn digest(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

pub fn state(target: Target, home: &Path) -> State {
    let dir = target.dir(home);
    let content = match fs::read_to_string(dir.join("SKILL.md")) {
        Ok(content) => content,
        Err(_) => return State::NotInstalled,
    };
    if content == SKILL_MD {
        return State::Current;
    }
    if !is_voxtype_skill(&content) {
        return State::Foreign;
    }
    match fs::read_to_string(dir.join(MARKER)) {
        Ok(recorded) if recorded.trim() == digest(&content) => State::Outdated,
        _ => State::Modified,
    }
}

/// Write this version's skill for `target`, replacing an older copy.
/// Returns the path written.
pub fn install(target: Target, home: &Path) -> io::Result<PathBuf> {
    let dir = target.dir(home);
    fs::create_dir_all(&dir)?;
    let path = dir.join("SKILL.md");
    // Write beside the target and rename, so an agent never reads half a file.
    let tmp = dir.join(".SKILL.md.tmp");
    fs::write(&tmp, SKILL_MD)?;
    fs::rename(&tmp, &path)?;
    fs::write(dir.join(MARKER), digest(SKILL_MD))?;
    Ok(path)
}

/// Remove the skill voxtype installed for `target`. Only `SKILL.md` and
/// voxtype's marker are deleted, and the directory only when nothing else is
/// left in it, so files a user added are kept. Callers ask before removing a
/// copy the user edited. Returns whether anything was removed.
pub fn remove(target: Target, home: &Path) -> io::Result<bool> {
    let dir = target.dir(home);
    let path = dir.join("SKILL.md");
    match fs::read_to_string(&path) {
        Ok(content) if is_voxtype_skill(&content) => {
            fs::remove_file(&path)?;
            let _ = fs::remove_file(dir.join(MARKER));
            // Fails harmlessly when the user keeps other files here.
            let _ = fs::remove_dir(&dir);
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Whether a SKILL.md is ours, by its frontmatter name, so `--remove` never
/// deletes an unrelated skill a user put at the same path.
fn is_voxtype_skill(content: &str) -> bool {
    let name = format!("name: {SKILL_NAME}");
    content.lines().take(10).any(|l| l.trim() == name)
}

pub struct CliArgs {
    pub status: bool,
    pub json: bool,
    pub remove: bool,
    pub force: bool,
    pub target: Option<String>,
}

/// Whether to replace or remove a copy in `state`. Copies the user edited, and
/// unrelated skills at the same path, need `--force` or a yes at the prompt;
/// without a terminal to ask on, they are left alone.
fn allowed(
    state: State,
    force: bool,
    interactive: bool,
    question: &str,
    confirm: &mut dyn FnMut(&str) -> bool,
) -> bool {
    !state.needs_consent() || force || (interactive && confirm(question))
}

fn confirm(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = io::stdout().flush();
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).is_ok()
        && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn consent_question(target: Target, home: &Path, state: State, action: &str) -> String {
    let path = target.dir(home).join("SKILL.md");
    match state {
        State::Foreign => format!(
            "{} holds a different skill, not voxtype-setup. {action} it?",
            path.display()
        ),
        _ => format!(
            "You have edited {}. {action} it and lose your changes?",
            path.display()
        ),
    }
}

fn left_alone(target: Target, home: &Path, state: State, action: &str) {
    let path = target.dir(home).join("SKILL.md");
    let why = if state == State::Foreign {
        "a different skill is there"
    } else {
        "it has your edits"
    };
    println!(
        "Left {} unchanged for {}: {why}. Run again with --force to {action} it.",
        path.display(),
        target.label()
    );
}

fn on_path(name: &str) -> bool {
    which::which(name).is_ok()
}

fn home() -> anyhow::Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine the home directory"))
}

/// Agents with a stale or missing copy, for the `voxtype setup check` hint.
/// Copies the user edited are theirs, so they are not reported.
pub fn needs_attention() -> Vec<(Target, State)> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    Target::ALL
        .into_iter()
        .filter(|t| t.detected(&home, &on_path))
        .map(|t| (t, state(t, &home)))
        .filter(|(_, s)| matches!(s, State::NotInstalled | State::Outdated))
        .collect()
}

/// Entry point for `voxtype setup skill`.
pub fn run(args: CliArgs) -> anyhow::Result<()> {
    let home = home()?;
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    let mut ask = confirm;
    let explicit =
        match args.target.as_deref() {
            Some(v) => Some(parse_targets(v).ok_or_else(|| {
                anyhow::anyhow!("unknown target '{v}': use claude, agents or all")
            })?),
            None => None,
        };

    if args.status {
        return print_status(&home, args.json);
    }

    if args.remove {
        let targets = explicit.unwrap_or_else(|| Target::ALL.to_vec());
        let mut removed_any = false;
        for t in targets {
            let before = state(t, &home);
            if before == State::Modified
                && !allowed(
                    before,
                    args.force,
                    interactive,
                    &consent_question(t, &home, before, "Remove"),
                    &mut ask,
                )
            {
                left_alone(t, &home, before, "remove");
                continue;
            }
            if remove(t, &home)? {
                println!(
                    "Removed the skill for {} ({})",
                    t.label(),
                    t.dir(&home).display()
                );
                removed_any = true;
            }
        }
        if !removed_any {
            println!("No voxtype-setup skill was installed.");
        }
        return Ok(());
    }

    let targets = match explicit {
        Some(t) => t,
        None => {
            let detected: Vec<Target> = Target::ALL
                .into_iter()
                .filter(|t| t.detected(&home, &on_path))
                .collect();
            if detected.is_empty() {
                anyhow::bail!(
                    "No coding agent found (looked for Claude Code, Codex, Gemini CLI, OpenCode, \
                     Cursor, Copilot and Goose).\n  \
                     Install for one anyway with: voxtype setup skill --target claude|agents|all"
                );
            }
            detected
        }
    };

    for t in targets {
        let before = state(t, &home);
        if before == State::Current {
            println!(
                "Already current for {}: {}",
                t.label(),
                t.dir(&home).display()
            );
            continue;
        }
        if !allowed(
            before,
            args.force,
            interactive,
            &consent_question(t, &home, before, "Replace"),
            &mut ask,
        ) {
            left_alone(t, &home, before, "replace");
            continue;
        }
        let path = install(t, &home)?;
        let verb = if before == State::NotInstalled {
            "Installed"
        } else {
            "Updated"
        };
        println!("{verb} the skill for {}: {}", t.label(), path.display());
    }
    println!(
        "\nAsk your agent to \"set up voxtype for this machine\". \
         Run `voxtype setup skill` again after upgrading voxtype to update it."
    );
    Ok(())
}

fn print_status(home: &Path, json: bool) -> anyhow::Result<()> {
    let rows: Vec<(Target, bool, State)> = Target::ALL
        .into_iter()
        .map(|t| (t, t.detected(home, &on_path), state(t, home)))
        .collect();

    if json {
        let list: Vec<serde_json::Value> = rows
            .iter()
            .map(|(t, detected, s)| {
                serde_json::json!({
                    "target": t.id(),
                    "agents": t.label(),
                    "detected": detected,
                    "path": t.dir(home).join("SKILL.md"),
                    "state": s.id(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }

    println!(
        "voxtype-setup skill (this version: {})\n",
        env!("CARGO_PKG_VERSION")
    );
    for (t, detected, s) in rows {
        let state = match s {
            State::Current => "current",
            State::Outdated => "outdated (run: voxtype setup skill)",
            State::Modified => "edited by you (voxtype setup skill asks before replacing it)",
            State::Foreign => "a different skill is at this path",
            State::NotInstalled => "not installed",
        };
        let found = if detected { "found" } else { "not found" };
        println!("  {:<10} {} [{found}]", t.id(), t.label());
        println!(
            "             {}: {state}",
            t.dir(home).join("SKILL.md").display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "voxtype-skill-test-{}-{}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn nothing_on_path(_: &str) -> bool {
        false
    }

    #[test]
    fn embedded_skill_has_the_expected_name() {
        assert!(is_voxtype_skill(SKILL_MD));
    }

    #[test]
    fn agents_are_detected_by_config_dir_or_binary() {
        let home = temp_home("detect");
        assert!(!Target::Claude.detected(&home, &nothing_on_path));
        assert!(!Target::Agents.detected(&home, &nothing_on_path));

        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::create_dir_all(home.join(".config/opencode")).unwrap();
        assert!(Target::Claude.detected(&home, &nothing_on_path));
        assert!(Target::Agents.detected(&home, &nothing_on_path));

        let fresh = temp_home("detect-bin");
        assert!(Target::Agents.detected(&fresh, &|b| b == "codex"));
        assert!(!Target::Claude.detected(&fresh, &|b| b == "codex"));
        fs::remove_dir_all(home).unwrap();
        fs::remove_dir_all(fresh).unwrap();
    }

    #[test]
    fn install_reports_current_then_outdated_after_a_change() {
        let home = temp_home("install");
        assert_eq!(state(Target::Claude, &home), State::NotInstalled);

        let path = install(Target::Claude, &home).unwrap();
        assert_eq!(path, home.join(".claude/skills/voxtype-setup/SKILL.md"));
        assert_eq!(state(Target::Claude, &home), State::Current);
        assert_eq!(state(Target::Agents, &home), State::NotInstalled);

        // An older voxtype wrote a different skill, and recorded it.
        let older = SKILL_MD.replace("Configuring", "Setting up");
        fs::write(&path, &older).unwrap();
        fs::write(path.with_file_name(MARKER), digest(&older)).unwrap();
        assert_eq!(state(Target::Claude, &home), State::Outdated);
        install(Target::Claude, &home).unwrap();
        assert_eq!(state(Target::Claude, &home), State::Current);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_copy_the_user_edited_is_modified_not_outdated() {
        let home = temp_home("edited");
        let path = install(Target::Claude, &home).unwrap();
        fs::write(
            &path,
            format!("{SKILL_MD}\n## My notes\nUse the large model.\n"),
        )
        .unwrap();
        assert_eq!(state(Target::Claude, &home), State::Modified);
        assert!(State::Modified.needs_consent());

        // No marker at all (copied in by hand) is treated as the user's too.
        fs::remove_file(path.with_file_name(MARKER)).unwrap();
        assert_eq!(state(Target::Claude, &home), State::Modified);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_different_skill_at_the_path_is_foreign() {
        let home = temp_home("foreign");
        let dir = Target::Agents.dir(&home);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), "---\nname: something-else\n---\n").unwrap();
        assert_eq!(state(Target::Agents, &home), State::Foreign);
        assert!(State::Foreign.needs_consent());
        assert!(!State::Outdated.needs_consent());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn edited_copies_are_replaced_only_with_force_or_a_yes() {
        let mut never = |_: &str| -> bool { panic!("must not prompt") };
        let mut yes = |_: &str| true;
        let mut no = |_: &str| false;

        // Unedited copies are replaced without asking.
        assert!(allowed(State::Outdated, false, true, "q", &mut never));
        assert!(allowed(State::NotInstalled, false, false, "q", &mut never));

        // Edited or foreign copies: --force, or a yes at the prompt.
        for s in [State::Modified, State::Foreign] {
            assert!(allowed(s, true, false, "q", &mut never));
            assert!(allowed(s, false, true, "q", &mut yes));
            assert!(!allowed(s, false, true, "q", &mut no));
            // Without a terminal there is nobody to ask, so leave it alone.
            assert!(!allowed(s, false, false, "q", &mut never));
        }
    }

    #[test]
    fn remove_keeps_other_files_and_foreign_skills() {
        let home = temp_home("remove");
        install(Target::Agents, &home).unwrap();
        let dir = Target::Agents.dir(&home);
        fs::write(dir.join("notes.txt"), "mine").unwrap();
        assert!(remove(Target::Agents, &home).unwrap());
        assert!(!dir.join("SKILL.md").exists());
        assert!(dir.join("notes.txt").exists(), "user files are kept");

        // A different skill at the same path is never deleted.
        fs::write(dir.join("SKILL.md"), "---\nname: something-else\n---\n").unwrap();
        assert!(!remove(Target::Agents, &home).unwrap());
        assert!(dir.join("SKILL.md").exists());

        assert!(!remove(Target::Claude, &home).unwrap(), "nothing installed");
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn targets_parse() {
        assert_eq!(parse_targets("claude"), Some(vec![Target::Claude]));
        assert_eq!(parse_targets("all"), Some(Target::ALL.to_vec()));
        assert_eq!(parse_targets("vim"), None);
    }
}
