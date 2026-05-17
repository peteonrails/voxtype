//! Detect `voxtype record` bindings declared in compositor configs.
//!
//! Useful when the user has the evdev listener disabled and is relying on
//! compositor-level keybindings to call voxtype. The Hotkey section's About
//! pane shows what bindings are wired up so users can verify their config
//! without leaving the TUI.
//!
//! Supports Hyprland, Sway, and Niri. Their config formats are parsed with
//! plain regex — we don't pull in a real KDL/Hyprland parser for what is
//! ultimately advisory output.
//!
//! # Compositors not yet covered
//!
//! - River: shell-script-based init; any function could call voxtype, so a
//!   simple grep would mostly produce false positives.
//! - GNOME / KDE: bindings live in dconf / kglobalshortcuts databases. Worth
//!   a follow-up but a different shape of detection.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Binding {
    pub compositor: &'static str,
    /// Human-readable key combo as written in the config (e.g. "SUPER+HOME").
    pub keys: String,
    /// Voxtype subcommand being bound (`record start`, `record cancel`,
    /// `meeting start`, `meeting stop`, …).
    pub action: String,
    /// Path to the file the binding came from, for reporting.
    pub source: PathBuf,
}

/// Format hint for a [`Suggestion`] — picked from the compositor that owns
/// the most existing bindings, falling back to Hyprland.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compositor {
    Hyprland,
    Sway,
    Niri,
}

impl Compositor {
    pub fn name(self) -> &'static str {
        match self {
            Compositor::Hyprland => "Hyprland",
            Compositor::Sway => "Sway",
            Compositor::Niri => "Niri",
        }
    }
}

/// One missing binding the user might want to add.
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub label: String,
    pub purpose: &'static str,
    pub config_lines: Vec<String>,
}

/// Pick the most likely compositor based on the bindings already detected,
/// or default to Hyprland.
pub fn dominant_compositor(detected: &[Binding]) -> Compositor {
    let mut hypr = 0;
    let mut sway = 0;
    let mut niri = 0;
    for b in detected {
        match b.compositor {
            "Hyprland" => hypr += 1,
            "Sway" => sway += 1,
            "Niri" => niri += 1,
            _ => {}
        }
    }
    if niri > hypr && niri > sway {
        Compositor::Niri
    } else if sway > hypr {
        Compositor::Sway
    } else {
        Compositor::Hyprland
    }
}

/// Look at the actions the user has already bound and suggest config snippets
/// for likely-missing ones (cancel, toggle, meeting start/stop). Suggested
/// keys come from a small candidate list, skipping any combo already bound to
/// another action in the user's compositor configs.
///
/// `streaming` indicates whether Parakeet streaming dictation is enabled. When
/// it is, this function only suggests toggle bindings: typing at the cursor
/// while a PTT key is held breaks libinput's held-key tracker on Hyprland,
/// Sway, and River, so streaming requires a toggle binding rather than the
/// usual press/release pair.
pub fn suggest_missing(detected: &[Binding], streaming: bool) -> Vec<Suggestion> {
    let comp = dominant_compositor(detected);
    let occupied = enumerate_occupied_keys(comp);
    let actions: std::collections::HashSet<&str> =
        detected.iter().map(|b| b.action.as_str()).collect();

    let has_start = actions.contains("record start");
    let has_stop = actions.contains("record stop");
    let has_toggle = actions.contains("record toggle");
    let has_cancel = actions.contains("record cancel");
    let has_meeting_start = actions.contains("meeting start");
    let has_meeting_stop = actions.contains("meeting stop");

    // Track keys we've already proposed in this batch so two suggestions don't
    // collide with each other.
    let mut taken: std::collections::HashSet<String> = occupied.clone();

    let mut suggestions = Vec::new();

    if streaming {
        if !has_toggle {
            suggestions.push(make_suggestion(
                comp,
                &mut taken,
                "Toggle (required for streaming)",
                "Streaming dictation types characters while you speak. A held \
                 PTT key would clobber libinput's held-key tracker on \
                 Hyprland/Sway/River, so streaming needs a single-press toggle.",
                Role::Toggle,
            ));
        }
    } else {
        if has_start && !has_stop {
            suggestions.push(make_suggestion(
                comp,
                &mut taken,
                "Stop (release of your PTT key)",
                "Without a stop binding, hold-to-record never finishes — voxtype \
                 will run until max_duration_secs hits.",
                Role::Stop,
            ));
        }
        if has_stop && !has_start {
            suggestions.push(make_suggestion(
                comp,
                &mut taken,
                "Start (press of your PTT key)",
                "You have a stop binding but no start — recording can't begin from \
                 your compositor.",
                Role::Start,
            ));
        }

        if !has_start && !has_stop && !has_toggle {
            suggestions.push(make_suggestion(
                comp,
                &mut taken,
                "Push-to-talk (start + stop pair)",
                "Hold the key while you speak; release to transcribe.",
                Role::PttPair,
            ));
            suggestions.push(make_suggestion(
                comp,
                &mut taken,
                "Toggle (single-key alternative)",
                "Press once to start, again to stop. Better for long dictations.",
                Role::Toggle,
            ));
        } else if !has_toggle && (has_start || has_stop) {
            suggestions.push(make_suggestion(
                comp,
                &mut taken,
                "Toggle (alternative to PTT)",
                "A single-key toggle bound to a different key gives you a \
                 long-dictation flow without competing with the PTT key.",
                Role::Toggle,
            ));
        }
    }

    if !has_cancel {
        suggestions.push(make_suggestion(
            comp,
            &mut taken,
            "Cancel (abort in-progress recording)",
            "Discards audio without transcribing — useful when you trip the \
             PTT key by accident or the wrong window has focus.",
            Role::Cancel,
        ));
    }

    if !has_meeting_start && !has_meeting_stop {
        suggestions.push(make_suggestion(
            comp,
            &mut taken,
            "Meeting mode (start + stop)",
            "Long-form recording with chunked transcription. Bind separate \
             keys so meeting capture doesn't collide with regular dictation.",
            Role::MeetingPair,
        ));
    } else if has_meeting_start && !has_meeting_stop {
        suggestions.push(make_suggestion(
            comp,
            &mut taken,
            "Meeting stop",
            "You have a meeting-start binding but no stop. Without it the \
             meeting only ends when you run `voxtype meeting stop` from the CLI.",
            Role::MeetingStop,
        ));
    } else if has_meeting_stop && !has_meeting_start {
        suggestions.push(make_suggestion(
            comp,
            &mut taken,
            "Meeting start",
            "You bound meeting stop but not start.",
            Role::MeetingStart,
        ));
    }

    suggestions
}

#[derive(Debug, Clone, Copy)]
enum Role {
    Start,
    Stop,
    Toggle,
    Cancel,
    PttPair,
    MeetingStart,
    MeetingStop,
    MeetingPair,
}

/// Candidate key combos in canonical form (uppercase, alphabetically sorted
/// modifiers, joined by '+'). Picked from in order; first non-occupied wins.
fn candidates_for(role: Role) -> &'static [&'static str] {
    match role {
        // PTT keys: typically modifier-free function/utility keys.
        Role::Start | Role::Stop | Role::PttPair => &[
            "F13", "F14", "F15", "F16", "HOME", "PAUSE", "SCROLLLOCK", "INSERT", "MENU",
        ],
        Role::Toggle => &[
            "SUPER+SPACE",
            "SUPER+SLASH",
            "SUPER+SEMICOLON",
            "SUPER+APOSTROPHE",
            "SUPER+BACKSLASH",
            "SUPER+COMMA",
            "SUPER+PERIOD",
        ],
        Role::Cancel => &[
            "SUPER+ESCAPE",
            "SUPER+BACKSPACE",
            "SUPER+DELETE",
            "CTRL+SUPER+ESCAPE",
        ],
        Role::MeetingStart => &[
            "SUPER+M",
            "CTRL+SUPER+M",
            "ALT+SUPER+M",
        ],
        Role::MeetingStop => &[
            "SHIFT+SUPER+M",
            "ALT+SUPER+M",
            "CTRL+SUPER+M",
        ],
        Role::MeetingPair => &[
            "SUPER+M",
            "CTRL+SUPER+M",
            "ALT+SUPER+M",
        ],
    }
}

fn make_suggestion(
    comp: Compositor,
    taken: &mut std::collections::HashSet<String>,
    label: &str,
    purpose: &'static str,
    role: Role,
) -> Suggestion {
    let candidates = candidates_for(role);
    let mut chosen: Option<&'static str> = None;
    for cand in candidates {
        if !taken.contains(*cand) {
            chosen = Some(cand);
            break;
        }
    }
    let key = chosen.unwrap_or(candidates[0]);
    let collision = chosen.is_none();
    if !collision {
        taken.insert(key.to_string());
    }
    let stop_key = if matches!(role, Role::MeetingPair) {
        // Pick a second key that doesn't collide with the start key just chosen.
        let stop_candidates = candidates_for(Role::MeetingStop);
        let mut second = None;
        for cand in stop_candidates {
            if !taken.contains(*cand) {
                second = Some(*cand);
                break;
            }
        }
        let chosen_stop = second.unwrap_or(stop_candidates[0]);
        if second.is_some() {
            taken.insert(chosen_stop.to_string());
        }
        Some(chosen_stop)
    } else {
        None
    };

    let mut config_lines = render_role(comp, role, key, stop_key.as_deref());
    if collision {
        config_lines.insert(
            0,
            "// All preferred candidates are already bound; pick a key that's free."
                .to_string(),
        );
    }

    Suggestion {
        label: label.to_string(),
        purpose,
        config_lines,
    }
}

/// Render one role into compositor-formatted binding lines, parameterized by
/// the chosen canonical key (and optional second key for paired roles).
fn render_role(
    comp: Compositor,
    role: Role,
    key: &str,
    second_key: Option<&str>,
) -> Vec<String> {
    match (comp, role) {
        (Compositor::Hyprland, Role::Start) => {
            vec![hyprland_bind("bindd", key, "Voxtype PTT (start)", "voxtype record start")]
        }
        (Compositor::Hyprland, Role::Stop) => {
            vec![hyprland_bind("bindrd", key, "Voxtype PTT (stop)", "voxtype record stop")]
        }
        (Compositor::Hyprland, Role::PttPair) => vec![
            hyprland_bind("bindd", key, "Voxtype PTT (start)", "voxtype record start"),
            hyprland_bind("bindrd", key, "Voxtype PTT (stop)", "voxtype record stop"),
        ],
        (Compositor::Hyprland, Role::Toggle) => {
            vec![hyprland_bind("bind", key, "Voxtype toggle", "voxtype record toggle")]
        }
        (Compositor::Hyprland, Role::Cancel) => {
            vec![hyprland_bind("bind", key, "Voxtype cancel", "voxtype record cancel")]
        }
        (Compositor::Hyprland, Role::MeetingStart) => {
            vec![hyprland_bind("bind", key, "Voxtype meeting start", "voxtype meeting start")]
        }
        (Compositor::Hyprland, Role::MeetingStop) => {
            vec![hyprland_bind("bind", key, "Voxtype meeting stop", "voxtype meeting stop")]
        }
        (Compositor::Hyprland, Role::MeetingPair) => vec![
            hyprland_bind("bind", key, "Voxtype meeting start", "voxtype meeting start"),
            hyprland_bind(
                "bind",
                second_key.unwrap_or("SHIFT+SUPER+M"),
                "Voxtype meeting stop",
                "voxtype meeting stop",
            ),
        ],

        (Compositor::Sway, Role::Start) => {
            vec![format!("bindsym {} exec voxtype record start", canonical_to_sway(key))]
        }
        (Compositor::Sway, Role::Stop) => vec![format!(
            "bindsym --release {} exec voxtype record stop",
            canonical_to_sway(key)
        )],
        (Compositor::Sway, Role::PttPair) => vec![
            format!("bindsym {} exec voxtype record start", canonical_to_sway(key)),
            format!(
                "bindsym --release {} exec voxtype record stop",
                canonical_to_sway(key)
            ),
        ],
        (Compositor::Sway, Role::Toggle) => vec![format!(
            "bindsym {} exec voxtype record toggle",
            canonical_to_sway(key)
        )],
        (Compositor::Sway, Role::Cancel) => vec![format!(
            "bindsym {} exec voxtype record cancel",
            canonical_to_sway(key)
        )],
        (Compositor::Sway, Role::MeetingStart) => vec![format!(
            "bindsym {} exec voxtype meeting start",
            canonical_to_sway(key)
        )],
        (Compositor::Sway, Role::MeetingStop) => vec![format!(
            "bindsym {} exec voxtype meeting stop",
            canonical_to_sway(key)
        )],
        (Compositor::Sway, Role::MeetingPair) => vec![
            format!(
                "bindsym {} exec voxtype meeting start",
                canonical_to_sway(key)
            ),
            format!(
                "bindsym {} exec voxtype meeting stop",
                canonical_to_sway(second_key.unwrap_or("SHIFT+SUPER+M"))
            ),
        ],

        (Compositor::Niri, Role::Start) => vec![format!(
            "{} {{ spawn \"voxtype\" \"record\" \"start\"; }}",
            canonical_to_niri(key)
        )],
        (Compositor::Niri, Role::Stop) => vec![format!(
            "// Niri does not bind on key release; consider Role::Toggle instead."
        )],
        (Compositor::Niri, Role::PttPair) => vec![
            format!(
                "{} {{ spawn \"voxtype\" \"record\" \"toggle\"; }}",
                canonical_to_niri(key)
            ),
            "// (Niri lacks key-release binds; use toggle in place of PTT.)"
                .to_string(),
        ],
        (Compositor::Niri, Role::Toggle) => vec![format!(
            "{} {{ spawn \"voxtype\" \"record\" \"toggle\"; }}",
            canonical_to_niri(key)
        )],
        (Compositor::Niri, Role::Cancel) => vec![format!(
            "{} {{ spawn \"voxtype\" \"record\" \"cancel\"; }}",
            canonical_to_niri(key)
        )],
        (Compositor::Niri, Role::MeetingStart) => vec![format!(
            "{} {{ spawn \"voxtype\" \"meeting\" \"start\"; }}",
            canonical_to_niri(key)
        )],
        (Compositor::Niri, Role::MeetingStop) => vec![format!(
            "{} {{ spawn \"voxtype\" \"meeting\" \"stop\"; }}",
            canonical_to_niri(key)
        )],
        (Compositor::Niri, Role::MeetingPair) => vec![
            format!(
                "{} {{ spawn \"voxtype\" \"meeting\" \"start\"; }}",
                canonical_to_niri(key)
            ),
            format!(
                "{} {{ spawn \"voxtype\" \"meeting\" \"stop\"; }}",
                canonical_to_niri(second_key.unwrap_or("SHIFT+SUPER+M"))
            ),
        ],
    }
}

fn hyprland_bind(directive: &str, canonical_key: &str, label: &str, cmd: &str) -> String {
    let (mods, key) = canonical_split(canonical_key);
    let mods_hypr = mods.replace('+', " ");
    if mods_hypr.is_empty() {
        format!("{} = , {}, {}, exec, {}", directive, key, label, cmd)
    } else {
        format!("{} = {}, {}, {}, exec, {}", directive, mods_hypr, key, label, cmd)
    }
}

/// Split "MODS+KEY" into ("MODS+SORTED", "KEY"). Modifiers come back '+'-joined.
fn canonical_split(canonical: &str) -> (String, String) {
    let parts: Vec<&str> = canonical.split('+').collect();
    if parts.len() == 1 {
        return (String::new(), parts[0].to_string());
    }
    let key = parts.last().copied().unwrap_or("").to_string();
    let mut mods: Vec<&str> = parts[..parts.len() - 1].to_vec();
    mods.sort();
    (mods.join("+"), key)
}

fn canonical_to_sway(canonical: &str) -> String {
    // Sway uses Mod4 for SUPER. Lowercase the key and capitalize modifiers.
    let (mods, key) = canonical_split(canonical);
    let mods = mods
        .split('+')
        .filter(|s| !s.is_empty())
        .map(|m| match m {
            "SUPER" => "Mod4",
            "ALT" => "Mod1",
            "CTRL" => "Ctrl",
            "SHIFT" => "Shift",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+");
    let sway_key = sway_key_name(&key);
    if mods.is_empty() {
        sway_key
    } else {
        format!("{}+{}", mods, sway_key)
    }
}

fn sway_key_name(canonical_key: &str) -> String {
    // Sway keysym names are lowercase and use specific words for some keys.
    match canonical_key {
        "SPACE" => "space".into(),
        "ESCAPE" => "Escape".into(),
        "BACKSPACE" => "BackSpace".into(),
        "DELETE" => "Delete".into(),
        "RETURN" | "ENTER" => "Return".into(),
        "PRINT" => "Print".into(),
        "PAUSE" => "Pause".into(),
        "INSERT" => "Insert".into(),
        "HOME" => "Home".into(),
        "END" => "End".into(),
        "MENU" => "Menu".into(),
        "SCROLLLOCK" => "Scroll_Lock".into(),
        "APOSTROPHE" => "apostrophe".into(),
        "SEMICOLON" => "semicolon".into(),
        "SLASH" => "slash".into(),
        "BACKSLASH" => "backslash".into(),
        "COMMA" => "comma".into(),
        "PERIOD" => "period".into(),
        // F1-F24 keep their casing.
        s if s.starts_with('F') && s[1..].chars().all(|c| c.is_ascii_digit()) => s.into(),
        // Single letters: lowercase.
        s if s.len() == 1 => s.to_lowercase(),
        s => s.into(),
    }
}

fn canonical_to_niri(canonical: &str) -> String {
    // Niri uses "Mod" for SUPER, capitalized modifiers separated by '+',
    // and human-cased key names.
    let (mods, key) = canonical_split(canonical);
    let mods = mods
        .split('+')
        .filter(|s| !s.is_empty())
        .map(|m| match m {
            "SUPER" => "Mod",
            "CTRL" => "Ctrl",
            "ALT" => "Alt",
            "SHIFT" => "Shift",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+");
    let niri_key = niri_key_name(&key);
    if mods.is_empty() {
        niri_key
    } else {
        format!("{}+{}", mods, niri_key)
    }
}

fn niri_key_name(canonical_key: &str) -> String {
    match canonical_key {
        "SPACE" => "Space".into(),
        "ESCAPE" => "Escape".into(),
        "BACKSPACE" => "BackSpace".into(),
        "DELETE" => "Delete".into(),
        "RETURN" | "ENTER" => "Return".into(),
        "INSERT" => "Insert".into(),
        "HOME" => "Home".into(),
        "END" => "End".into(),
        "PAUSE" => "Pause".into(),
        "MENU" => "Menu".into(),
        "SCROLLLOCK" => "Scroll_Lock".into(),
        "APOSTROPHE" => "Apostrophe".into(),
        "SEMICOLON" => "Semicolon".into(),
        "SLASH" => "Slash".into(),
        "BACKSLASH" => "Backslash".into(),
        "COMMA" => "Comma".into(),
        "PERIOD" => "Period".into(),
        s if s.starts_with('F') && s[1..].chars().all(|c| c.is_ascii_digit()) => s.into(),
        s => s.into(),
    }
}

/// Walk all compositor configs and return the canonical-form set of every
/// key combo bound to anything, regardless of action. Used to make
/// suggestions skip combos already in use.
fn enumerate_occupied_keys(_comp: Compositor) -> std::collections::HashSet<String> {
    // We collect keys from every compositor we can find — better to over-skip
    // candidates than to clash with a user's existing binding because we
    // assumed the wrong compositor.
    let mut out = std::collections::HashSet::new();
    let home = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => return out,
    };
    enumerate_hyprland_keys(&home, &mut out);
    enumerate_sway_keys(&home, &mut out);
    enumerate_niri_keys(&home, &mut out);
    out
}

fn enumerate_hyprland_keys(home: &Path, out: &mut std::collections::HashSet<String>) {
    let dir = home.join(".config/hypr");
    let Ok(entries) = fs::read_dir(&dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("conf") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                continue;
            }
            let Some((lhs, rhs)) = trimmed.split_once('=') else { continue };
            if !lhs.trim().starts_with("bind") {
                continue;
            }
            let parts: Vec<&str> = rhs.split(',').map(str::trim).collect();
            if parts.len() < 2 {
                continue;
            }
            let mods = parts[0];
            let key = parts[1];
            if key.is_empty() {
                continue;
            }
            out.insert(canonicalize_hyprland(mods, key));
        }
    }
}

fn enumerate_sway_keys(home: &Path, out: &mut std::collections::HashSet<String>) {
    let mut paths: Vec<PathBuf> = Vec::new();
    let main = home.join(".config/sway/config");
    if main.exists() {
        paths.push(main);
    }
    if let Ok(entries) = fs::read_dir(home.join(".config/sway/config.d")) {
        for entry in entries.flatten() {
            paths.push(entry.path());
        }
    }
    for path in paths {
        let Ok(text) = fs::read_to_string(&path) else { continue };
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let Some(head) = parts.next() else { continue };
            if head != "bindsym" && head != "bindcode" {
                continue;
            }
            let mut rest: Vec<&str> = parts.collect();
            while let Some(first) = rest.first() {
                if first.starts_with("--") {
                    rest.remove(0);
                } else {
                    break;
                }
            }
            let Some(combo) = rest.first() else { continue };
            out.insert(canonicalize_sway(combo));
        }
    }
}

fn enumerate_niri_keys(home: &Path, out: &mut std::collections::HashSet<String>) {
    let path = home.join(".config/niri/config.kdl");
    let Ok(text) = fs::read_to_string(&path) else { return };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        let Some((keys, _)) = trimmed.split_once('{') else { continue };
        let keys = keys.trim();
        if keys.is_empty() {
            continue;
        }
        if !keys
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '_')
        {
            continue;
        }
        out.insert(canonicalize_niri(keys));
    }
}

fn canonicalize_hyprland(mods: &str, key: &str) -> String {
    let mut parts: Vec<String> = mods
        .split_whitespace()
        .map(|m| m.to_uppercase())
        .filter(|s| !s.is_empty())
        .collect();
    parts.sort();
    parts.push(key.to_uppercase());
    parts.join("+")
}

fn canonicalize_sway(combo: &str) -> String {
    let mut parts: Vec<String> = combo
        .split('+')
        .map(|m| match m.to_lowercase().as_str() {
            "mod4" => "SUPER".to_string(),
            "mod1" => "ALT".to_string(),
            "ctrl" | "control" => "CTRL".to_string(),
            "shift" => "SHIFT".to_string(),
            other => other.to_uppercase(),
        })
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    let key = parts.pop().unwrap_or_default();
    let key = sway_key_canon(&key);
    parts.sort();
    parts.push(key);
    parts.join("+")
}

fn sway_key_canon(s: &str) -> String {
    // Map sway key names back to canonical (uppercase) form.
    match s.to_lowercase().as_str() {
        "space" => "SPACE".into(),
        "escape" => "ESCAPE".into(),
        "backspace" => "BACKSPACE".into(),
        "return" | "enter" => "RETURN".into(),
        "delete" => "DELETE".into(),
        "scroll_lock" => "SCROLLLOCK".into(),
        "apostrophe" => "APOSTROPHE".into(),
        "semicolon" => "SEMICOLON".into(),
        "slash" => "SLASH".into(),
        "backslash" => "BACKSLASH".into(),
        "comma" => "COMMA".into(),
        "period" => "PERIOD".into(),
        other => other.to_uppercase(),
    }
}

fn canonicalize_niri(combo: &str) -> String {
    let mut parts: Vec<String> = combo
        .split('+')
        .map(|m| match m.to_lowercase().as_str() {
            "mod" => "SUPER".to_string(),
            "ctrl" => "CTRL".to_string(),
            "alt" => "ALT".to_string(),
            "shift" => "SHIFT".to_string(),
            other => other.to_uppercase(),
        })
        .collect();
    if parts.is_empty() {
        return String::new();
    }
    let key = parts.pop().unwrap_or_default();
    parts.sort();
    parts.push(key);
    parts.join("+")
}

pub fn detect() -> Vec<Binding> {
    let mut out = Vec::new();
    let home = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => return out,
    };

    detect_hyprland(&home, &mut out);
    detect_sway(&home, &mut out);
    detect_niri(&home, &mut out);
    out
}

fn detect_hyprland(home: &Path, out: &mut Vec<Binding>) {
    let dir = home.join(".config/hypr");
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("conf") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            if let Some(b) = parse_hyprland_line(line, &path) {
                out.push(b);
            }
        }
    }
}

/// Hyprland `bindd? = MODS, KEY, NAME, exec, voxtype SUBCMD ACTION` lines
/// (and `bindrd?`, `bindl`, `bindel`, `binde`, `bindle`, …).
fn parse_hyprland_line(line: &str, source: &Path) -> Option<Binding> {
    let trimmed = line.trim();
    if trimmed.starts_with('#') {
        return None;
    }
    let (lhs, rhs) = trimmed.split_once('=')?;
    let lhs = lhs.trim();
    if !lhs.starts_with("bind") {
        return None;
    }
    if !rhs.contains("voxtype") {
        return None;
    }
    // Split by commas; Hyprland tolerates whitespace.
    let parts: Vec<&str> = rhs.split(',').map(str::trim).collect();
    if parts.len() < 4 {
        return None;
    }
    let mods = parts[0];
    let key = parts[1];
    let cmd = parts.last().copied().unwrap_or("");
    let action = action_from_command(cmd)?;
    let keys = if mods.is_empty() {
        key.to_string()
    } else {
        format!("{}+{}", mods, key)
    };
    Some(Binding {
        compositor: "Hyprland",
        keys,
        action,
        source: source.to_path_buf(),
    })
}

fn detect_sway(home: &Path, out: &mut Vec<Binding>) {
    let main = home.join(".config/sway/config");
    if main.exists() {
        if let Ok(text) = fs::read_to_string(&main) {
            for line in text.lines() {
                if let Some(b) = parse_sway_line(line, &main) {
                    out.push(b);
                }
            }
        }
    }
    let conf_d = home.join(".config/sway/config.d");
    if let Ok(entries) = fs::read_dir(&conf_d) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for line in text.lines() {
                if let Some(b) = parse_sway_line(line, &path) {
                    out.push(b);
                }
            }
        }
    }
}

/// Sway `bindsym MOD+KEY exec voxtype SUBCMD ACTION` (or `bindcode`).
fn parse_sway_line(line: &str, source: &Path) -> Option<Binding> {
    let trimmed = line.trim();
    if trimmed.starts_with('#') {
        return None;
    }
    if !trimmed.contains("voxtype") {
        return None;
    }
    let mut parts = trimmed.split_whitespace();
    let head = parts.next()?;
    if head != "bindsym" && head != "bindcode" {
        return None;
    }
    // Skip optional `--release` and similar flags.
    let mut rest: Vec<&str> = parts.collect();
    while let Some(first) = rest.first() {
        if first.starts_with("--") {
            rest.remove(0);
        } else {
            break;
        }
    }
    let keys = rest.first()?.to_string();
    // Find `exec` and look at what comes after `voxtype record`.
    let cmd_start = rest.iter().position(|w| *w == "exec")? + 1;
    let cmd = rest[cmd_start..].join(" ");
    let action = action_from_command(&cmd)?;
    Some(Binding {
        compositor: "Sway",
        keys,
        action,
        source: source.to_path_buf(),
    })
}

fn detect_niri(home: &Path, out: &mut Vec<Binding>) {
    let path = home.join(".config/niri/config.kdl");
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    for line in text.lines() {
        if let Some(b) = parse_niri_line(line, &path) {
            out.push(b);
        }
    }
}

/// Niri's KDL `binds { Mod+Key { spawn "voxtype" "record" "ACTION"; } }`.
/// We only handle single-line bindings, which is the common case.
fn parse_niri_line(line: &str, source: &Path) -> Option<Binding> {
    let trimmed = line.trim();
    if trimmed.starts_with("//") {
        return None;
    }
    if !trimmed.contains("voxtype") || !trimmed.contains("spawn") {
        return None;
    }
    // Form: `Mod+Key { spawn "voxtype" "record" "ACTION"; }`.
    let (keys, rest) = trimmed.split_once('{')?;
    let keys = keys.trim();
    if keys.is_empty() {
        return None;
    }
    // Pull the quoted args after `spawn`.
    let spawn_idx = rest.find("spawn")?;
    let args_part = &rest[spawn_idx + "spawn".len()..];
    let mut quoted: Vec<String> = Vec::new();
    let mut chars = args_part.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            let mut buf = String::new();
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                buf.push(c);
            }
            quoted.push(buf);
        }
    }
    if quoted.first().map(|s| s.as_str()) != Some("voxtype") {
        return None;
    }
    let subcmd = quoted.get(1)?.clone();
    let leaf = quoted.get(2)?.clone();
    let action = format!("{} {}", subcmd, leaf);
    if !is_known_action(&action) {
        return None;
    }
    Some(Binding {
        compositor: "Niri",
        keys: keys.to_string(),
        action,
        source: source.to_path_buf(),
    })
}

fn action_from_command(cmd: &str) -> Option<String> {
    // Look for `voxtype <subcmd> <leaf>` in the command line.
    let lc = cmd.to_lowercase();
    let idx = lc.find("voxtype")?;
    let after = &cmd[idx + "voxtype".len()..];
    let mut iter = after.split_whitespace();
    let subcmd = iter
        .next()?
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string();
    let leaf = iter
        .next()?
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string();
    let action = format!("{} {}", subcmd, leaf);
    if is_known_action(&action) {
        Some(action)
    } else {
        None
    }
}

fn is_known_action(action: &str) -> bool {
    matches!(
        action,
        "record start"
            | "record stop"
            | "record toggle"
            | "record cancel"
            | "meeting start"
            | "meeting stop"
            | "meeting pause"
            | "meeting resume"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn dummy_path() -> &'static Path {
        Path::new("/tmp/dummy.conf")
    }

    #[test]
    fn parses_hyprland_bindd() {
        let line = "bindd  = , HOME, Voxtype PTT (start), exec, voxtype record start";
        let b = parse_hyprland_line(line, dummy_path()).unwrap();
        assert_eq!(b.compositor, "Hyprland");
        assert_eq!(b.keys, "HOME");
        assert_eq!(b.action, "record start");
    }

    #[test]
    fn parses_hyprland_bindrd_with_mod() {
        let line = "bindrd = SUPER, F13, Stop, exec, voxtype record stop";
        let b = parse_hyprland_line(line, dummy_path()).unwrap();
        assert_eq!(b.keys, "SUPER+F13");
        assert_eq!(b.action, "record stop");
    }

    #[test]
    fn parses_hyprland_meeting_start() {
        let line = "bind = SUPER, M, Meeting start, exec, voxtype meeting start";
        let b = parse_hyprland_line(line, dummy_path()).unwrap();
        assert_eq!(b.action, "meeting start");
    }

    #[test]
    fn skips_hyprland_comments_and_unrelated() {
        assert!(parse_hyprland_line("# bind = , HOME, ..., exec, voxtype record start", dummy_path()).is_none());
        assert!(parse_hyprland_line("bind = , HOME, ..., exec, alacritty", dummy_path()).is_none());
    }

    #[test]
    fn parses_sway_bindsym() {
        let line = "bindsym Mod4+Home exec voxtype record toggle";
        let b = parse_sway_line(line, dummy_path()).unwrap();
        assert_eq!(b.compositor, "Sway");
        assert_eq!(b.keys, "Mod4+Home");
        assert_eq!(b.action, "record toggle");
    }

    #[test]
    fn parses_sway_with_release_flag() {
        let line = "bindsym --release Mod4+Home exec voxtype record stop";
        let b = parse_sway_line(line, dummy_path()).unwrap();
        assert_eq!(b.keys, "Mod4+Home");
        assert_eq!(b.action, "record stop");
    }

    #[test]
    fn parses_niri_spawn() {
        let line = r#"    Mod+Home { spawn "voxtype" "record" "start"; }"#;
        let b = parse_niri_line(line, dummy_path()).unwrap();
        assert_eq!(b.compositor, "Niri");
        assert_eq!(b.keys, "Mod+Home");
        assert_eq!(b.action, "record start");
    }

    #[test]
    fn parses_niri_meeting() {
        let line = r#"Mod+M { spawn "voxtype" "meeting" "start"; }"#;
        let b = parse_niri_line(line, dummy_path()).unwrap();
        assert_eq!(b.action, "meeting start");
    }

    #[test]
    fn suggests_cancel_when_only_ptt_bound() {
        let detected = vec![Binding {
            compositor: "Hyprland",
            keys: "HOME".into(),
            action: "record start".into(),
            source: PathBuf::from("/dev/null"),
        }, Binding {
            compositor: "Hyprland",
            keys: "HOME".into(),
            action: "record stop".into(),
            source: PathBuf::from("/dev/null"),
        }];
        let labels: Vec<_> = suggest_missing(&detected, false)
            .iter()
            .map(|s| s.label.clone())
            .collect();
        assert!(labels.iter().any(|l| l.contains("Cancel")));
        assert!(labels.iter().any(|l| l.contains("Toggle")));
        assert!(labels.iter().any(|l| l.contains("Meeting")));
    }

    #[test]
    fn suggest_missing_streaming_only_offers_toggle() {
        let detected: Vec<Binding> = vec![];
        let suggestions = suggest_missing(&detected, true);
        let labels: Vec<_> = suggestions.iter().map(|s| s.label.clone()).collect();
        // No PTT pair, no Start, no Stop suggestions when streaming is on.
        assert!(!labels.iter().any(|l| l.contains("Push-to-talk")));
        assert!(!labels.iter().any(|l| l == "Start (press of your PTT key)"));
        assert!(!labels.iter().any(|l| l == "Stop (release of your PTT key)"));
        // Toggle is the only record-related suggestion offered.
        assert!(labels.iter().any(|l| l.contains("Toggle")));
    }

    #[test]
    fn dominant_compositor_picks_majority() {
        let bindings = vec![
            Binding {
                compositor: "Sway",
                keys: "k".into(),
                action: "record start".into(),
                source: PathBuf::new(),
            },
            Binding {
                compositor: "Sway",
                keys: "k".into(),
                action: "record stop".into(),
                source: PathBuf::new(),
            },
            Binding {
                compositor: "Hyprland",
                keys: "k".into(),
                action: "record toggle".into(),
                source: PathBuf::new(),
            },
        ];
        assert_eq!(dominant_compositor(&bindings), Compositor::Sway);
    }

    #[test]
    fn dominant_compositor_empty_defaults_to_hyprland() {
        assert_eq!(dominant_compositor(&[]), Compositor::Hyprland);
    }

    #[test]
    fn canonicalize_hyprland_sorts_modifiers() {
        assert_eq!(canonicalize_hyprland("SUPER SHIFT", "M"), "SHIFT+SUPER+M");
        assert_eq!(canonicalize_hyprland("", "HOME"), "HOME");
        assert_eq!(canonicalize_hyprland("super", "f13"), "SUPER+F13");
    }

    #[test]
    fn canonicalize_sway_normalizes_mod4_and_keys() {
        assert_eq!(canonicalize_sway("Mod4+space"), "SUPER+SPACE");
        assert_eq!(canonicalize_sway("Mod4+Shift+m"), "SHIFT+SUPER+M");
        assert_eq!(canonicalize_sway("Escape"), "ESCAPE");
    }

    #[test]
    fn canonicalize_niri_normalizes_mod_word() {
        assert_eq!(canonicalize_niri("Mod+Shift+M"), "SHIFT+SUPER+M");
    }

    #[test]
    fn make_suggestion_skips_first_candidate_if_taken() {
        let mut taken: std::collections::HashSet<String> =
            ["F13", "F14"].iter().map(|s| s.to_string()).collect();
        let s = make_suggestion(
            Compositor::Hyprland,
            &mut taken,
            "PTT",
            "test",
            Role::PttPair,
        );
        assert!(
            s.config_lines.iter().any(|l| l.contains("F15")),
            "expected F15 in lines: {:?}",
            s.config_lines
        );
        assert!(
            !s.config_lines.iter().any(|l| l.contains(" F13 ") || l.contains(" F14 ")),
            "lines should not include F13/F14: {:?}",
            s.config_lines
        );
    }

    #[test]
    fn make_suggestion_warns_when_all_candidates_taken() {
        let mut taken: std::collections::HashSet<String> = candidates_for(Role::Cancel)
            .iter()
            .map(|s| s.to_string())
            .collect();
        let s = make_suggestion(
            Compositor::Hyprland,
            &mut taken,
            "Cancel",
            "test",
            Role::Cancel,
        );
        assert!(
            s.config_lines
                .iter()
                .any(|l| l.contains("preferred candidates are already bound")),
            "expected collision warning, got: {:?}",
            s.config_lines
        );
    }

    #[test]
    fn sway_render_uses_release_for_stop() {
        let mut taken = std::collections::HashSet::new();
        let s = make_suggestion(
            Compositor::Sway,
            &mut taken,
            "Stop",
            "test",
            Role::Stop,
        );
        assert!(s.config_lines.iter().any(|l| l.contains("--release")));
    }

    #[test]
    fn niri_pttpair_falls_back_to_toggle() {
        let mut taken = std::collections::HashSet::new();
        let s = make_suggestion(
            Compositor::Niri,
            &mut taken,
            "PTT",
            "test",
            Role::PttPair,
        );
        assert!(s.config_lines.iter().any(|l| l.contains("\"toggle\"")));
    }

    #[test]
    fn niri_skips_other_spawn_lines() {
        let line = r#"    Mod+T { spawn "alacritty"; }"#;
        assert!(parse_niri_line(line, dummy_path()).is_none());
    }

    #[test]
    fn niri_skips_comments() {
        let line = r#"// Mod+Home { spawn "voxtype" "record" "start"; }"#;
        assert!(parse_niri_line(line, dummy_path()).is_none());
    }

    #[test]
    fn rejects_unknown_action() {
        let line = "bindd = , HOME, ..., exec, voxtype record dance";
        assert!(parse_hyprland_line(line, dummy_path()).is_none());
    }
}
