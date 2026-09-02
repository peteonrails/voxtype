//! Omarchy theme integration.
//!
//! On startup, both OSD frontends read the active Omarchy theme and map it
//! to a [`Palette`] used by the renderer. The active theme usually lives at
//! `~/.local/state/omarchy/current/theme/colors.toml`, with older installs
//! using `~/.config/omarchy/current/theme/colors.toml`. The colors file has a
//! flat structure: `background`, `foreground`, `accent`, plus the ANSI palette
//! `color0`..=`color15`.
//!
//! Mapping:
//!
//! - `accent` → waveform fill
//! - `background` → window background (alpha kept from fallback)
//! - `foreground` → held-peak tick
//! - `color2` (ANSI green) → meter low zone
//! - `color3` (ANSI yellow) → meter mid zone
//! - `color1` (ANSI red) → meter high zone
//!
//! Themes whose ANSI red/green/yellow are off-spec (e.g. the "aether" theme
//! maps red to a tan) inherit the theme designer's choice — that's the
//! point of theming.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Deserialize;

use crate::osd::visual::{Color, Palette};

/// Candidate Omarchy "current theme" directories, ordered newest to oldest.
pub fn omarchy_theme_dirs() -> Option<Vec<PathBuf>> {
    let home = std::env::var_os("HOME")?;
    let home = PathBuf::from(home);
    Some(vec![
        home.join(".local/state/omarchy/current/theme"),
        home.join(".config/omarchy/current/theme"),
    ])
}

/// Omarchy "current" directories (the parents holding the `theme` symlink)
/// that actually exist on this system.
pub fn omarchy_current_dirs() -> Vec<PathBuf> {
    omarchy_theme_dirs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|dir| dir.parent().map(Path::to_path_buf))
        .filter(|dir| dir.is_dir())
        .collect()
}

/// Preferred Omarchy "current theme" directory.
pub fn omarchy_theme_dir() -> Option<PathBuf> {
    omarchy_theme_dirs()?.into_iter().next()
}

#[derive(Deserialize, Default)]
struct OmarchyColors {
    background: Option<String>,
    foreground: Option<String>,
    accent: Option<String>,
    color1: Option<String>,
    color2: Option<String>,
    color3: Option<String>,
}

/// Parse a `#RRGGBB` hex color into a [`Color`] with full alpha.
fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&s[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&s[4..6], 16).ok()? as f32 / 255.0;
    Some(Color::rgb(r, g, b))
}

/// Load the palette from the active Omarchy theme.
///
/// Falls back to [`Palette::fallback`] when the theme directory is missing,
/// the colors file is unreadable, or the TOML doesn't parse. Per-field
/// fallbacks apply too: a theme that only defines `accent` keeps the
/// fallback values for everything else.
pub fn load_palette() -> Palette {
    let Some(dirs) = omarchy_theme_dirs() else {
        return Palette::fallback();
    };

    for path in dirs.into_iter().map(|dir| dir.join("colors.toml")) {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(parsed) = toml::from_str(&content) else {
            continue;
        };
        return palette_from(parsed);
    }

    Palette::fallback()
}

fn palette_from(c: OmarchyColors) -> Palette {
    let fb = Palette::fallback();
    let bg_alpha = fb.background.a;
    Palette {
        background: c
            .background
            .as_deref()
            .and_then(parse_hex)
            .map(|c| c.with_alpha(bg_alpha))
            .unwrap_or(fb.background),
        accent: c.accent.as_deref().and_then(parse_hex).unwrap_or(fb.accent),
        meter_low: c
            .color2
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fb.meter_low),
        meter_mid: c
            .color3
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fb.meter_mid),
        meter_high: c
            .color1
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fb.meter_high),
        foreground: c
            .foreground
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fb.foreground),
    }
}

/// Theme snapshot: captures the palette at construction.
///
/// For live reload on Omarchy theme switches, see [`watch_current_theme`].
pub struct ThemeWatcher {
    palette: Palette,
}

impl ThemeWatcher {
    pub fn new() -> Self {
        Self {
            palette: load_palette(),
        }
    }

    /// Current palette. Cheap to call every frame.
    pub fn palette(&self) -> Palette {
        self.palette
    }
}

impl Default for ThemeWatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Quiet period after a filesystem event before the change callback fires.
/// `omarchy-theme-set` touches many files in one switch; the debounce
/// coalesces the burst into a single reload.
pub const THEME_DEBOUNCE: Duration = Duration::from_millis(300);

enum WatchMsg {
    Fs,
    Stop,
}

/// Keeps a live Omarchy theme watch running. Dropping the handle stops the
/// watcher thread.
pub struct ThemeWatchHandle {
    tx: mpsc::Sender<WatchMsg>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for ThemeWatchHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(WatchMsg::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Watch the active Omarchy theme and invoke `on_change` (debounced) each
/// time it changes. Returns `None` when no Omarchy install is present.
pub fn watch_current_theme<F>(on_change: F) -> Option<ThemeWatchHandle>
where
    F: FnMut() + Send + 'static,
{
    watch_omarchy_dirs(omarchy_current_dirs(), THEME_DEBOUNCE, on_change)
}

/// Watch the given Omarchy "current" directories for theme changes.
///
/// Two kinds of change are covered: `omarchy-theme-set` flips the `theme`
/// symlink inside a "current" directory (caught by the non-recursive watch
/// on the parent), and theme files can be edited in place (caught by a
/// watch on the resolved theme directory, re-pinned after every change so
/// a symlink flip moves it to the new target).
///
/// Bursts of events are debounced: `on_change` fires once per quiet period
/// of `debounce`. Returns `None` when none of `current_dirs` exist or no
/// watch could be established; errors never panic.
pub fn watch_omarchy_dirs<F>(
    current_dirs: Vec<PathBuf>,
    debounce: Duration,
    mut on_change: F,
) -> Option<ThemeWatchHandle>
where
    F: FnMut() + Send + 'static,
{
    let parents: Vec<PathBuf> = current_dirs.into_iter().filter(|d| d.is_dir()).collect();
    if parents.is_empty() {
        return None;
    }

    let (tx, rx) = mpsc::channel::<WatchMsg>();
    let event_tx = tx.clone();
    let mut watcher =
        match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if res.is_ok() {
                let _ = event_tx.send(WatchMsg::Fs);
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "failed to create Omarchy theme watcher");
                return None;
            }
        };

    let mut watching = 0usize;
    for parent in &parents {
        match watcher.watch(parent, RecursiveMode::NonRecursive) {
            Ok(()) => watching += 1,
            Err(e) => {
                tracing::warn!(dir = %parent.display(), error = %e, "failed to watch Omarchy dir")
            }
        }
    }
    if watching == 0 {
        return None;
    }
    let mut resolved = watch_theme_targets(&mut watcher, &parents, &[]);

    let thread = thread::spawn(move || loop {
        match rx.recv() {
            Ok(WatchMsg::Stop) | Err(_) => break,
            Ok(WatchMsg::Fs) => {
                loop {
                    match rx.recv_timeout(debounce) {
                        Ok(WatchMsg::Fs) => continue,
                        Ok(WatchMsg::Stop) => return,
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
                resolved = watch_theme_targets(&mut watcher, &parents, &resolved);
                on_change();
            }
        }
    });

    Some(ThemeWatchHandle {
        tx,
        thread: Some(thread),
    })
}

/// (Re-)pin the watcher to the theme directories the `theme` symlinks
/// currently resolve to, dropping watches on directories no longer active.
fn watch_theme_targets(
    watcher: &mut RecommendedWatcher,
    parents: &[PathBuf],
    old: &[PathBuf],
) -> Vec<PathBuf> {
    let resolved: Vec<PathBuf> = parents
        .iter()
        .filter_map(|p| fs::canonicalize(p.join("theme")).ok())
        .collect();
    for stale in old.iter().filter(|dir| !resolved.contains(dir)) {
        let _ = watcher.unwatch(stale);
    }
    for dir in resolved.iter().filter(|dir| !old.contains(*dir)) {
        if let Err(e) = watcher.watch(dir, RecursiveMode::NonRecursive) {
            tracing::debug!(dir = %dir.display(), error = %e, "failed to watch theme dir");
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_dir_resolves_under_home() {
        std::env::set_var("HOME", "/tmp/fakehome");
        let p = omarchy_theme_dir().unwrap();
        assert!(p.ends_with(".local/state/omarchy/current/theme"));
    }

    #[test]
    fn theme_dirs_include_legacy_config_fallback() {
        std::env::set_var("HOME", "/tmp/fakehome");
        let dirs = omarchy_theme_dirs().unwrap();
        assert!(dirs[0].ends_with(".local/state/omarchy/current/theme"));
        assert!(dirs[1].ends_with(".config/omarchy/current/theme"));
    }

    #[test]
    fn missing_theme_dir_yields_fallback() {
        std::env::set_var("HOME", "/tmp/this-dir-should-not-exist-voxtype-test");
        assert_eq!(load_palette(), Palette::fallback());
    }

    #[test]
    fn parse_hex_basic() {
        let c = parse_hex("#6E89C2").unwrap();
        assert!((c.r - 0x6E as f32 / 255.0).abs() < 1e-6);
        assert!((c.g - 0x89 as f32 / 255.0).abs() < 1e-6);
        assert!((c.b - 0xC2 as f32 / 255.0).abs() < 1e-6);
        assert_eq!(c.a, 1.0);
    }

    #[test]
    fn parse_hex_no_hash_prefix() {
        let c = parse_hex("121515").unwrap();
        assert!((c.r - 0x12 as f32 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn parse_hex_rejects_short_or_invalid() {
        assert!(parse_hex("#FFF").is_none());
        assert!(parse_hex("#ZZZZZZ").is_none());
        assert!(parse_hex("").is_none());
    }

    #[test]
    fn palette_from_aether_sample() {
        // Real values from ~/.config/omarchy/themes/aether/colors.toml
        let toml_src = r##"
            accent = "#6E89C2"
            background = "#121515"
            foreground = "#FCFBF8"
            color1 = "#A48364"
            color2 = "#F8E7AE"
            color3 = "#FEE88B"
        "##;
        let c: OmarchyColors = toml::from_str(toml_src).unwrap();
        let p = palette_from(c);
        assert_eq!(p.accent, parse_hex("#6E89C2").unwrap());
        // Background keeps the fallback alpha (translucent OSD).
        let fb_alpha = Palette::fallback().background.a;
        assert!((p.background.a - fb_alpha).abs() < 1e-6);
        assert_eq!(p.meter_high, parse_hex("#A48364").unwrap());
        assert_eq!(p.meter_low, parse_hex("#F8E7AE").unwrap());
        assert_eq!(p.meter_mid, parse_hex("#FEE88B").unwrap());
    }

    #[test]
    fn palette_from_partial_inherits_fallback() {
        // Only accent defined; everything else stays as fallback.
        let toml_src = r##"accent = "#6E89C2""##;
        let c: OmarchyColors = toml::from_str(toml_src).unwrap();
        let p = palette_from(c);
        let fb = Palette::fallback();
        assert_eq!(p.accent, parse_hex("#6E89C2").unwrap());
        assert_eq!(p.background, fb.background);
        assert_eq!(p.meter_low, fb.meter_low);
    }

    #[test]
    fn watcher_uses_loaded_palette() {
        // We can't predict the user's theme here, but at minimum the watcher
        // should hold whatever load_palette() returned at construction.
        let w = ThemeWatcher::new();
        assert_eq!(w.palette(), load_palette());
    }

    /// Test debounce window. Generous relative to inotify latency so a burst
    /// unambiguously coalesces, small enough to keep the suite fast.
    const DEBOUNCE: Duration = Duration::from_millis(250);

    /// Build a fake Omarchy layout: `current/theme` symlinks to `themes/a`,
    /// with `themes/b` available as a flip target.
    fn fake_omarchy() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let theme_a = tmp.path().join("themes/a");
        let theme_b = tmp.path().join("themes/b");
        let current = tmp.path().join("current");
        fs::create_dir_all(&theme_a).unwrap();
        fs::create_dir_all(&theme_b).unwrap();
        fs::create_dir_all(&current).unwrap();
        fs::write(theme_a.join("colors.toml"), "accent = \"#111111\"\n").unwrap();
        fs::write(theme_b.join("colors.toml"), "accent = \"#222222\"\n").unwrap();
        std::os::unix::fs::symlink(&theme_a, current.join("theme")).unwrap();
        (tmp, current, theme_a, theme_b)
    }

    /// Flip `current/theme` the way `ln -nsf` does: tmp symlink + rename.
    fn flip_theme(current: &Path, target: &Path) {
        let tmp = current.join(".theme.tmp");
        std::os::unix::fs::symlink(target, &tmp).unwrap();
        fs::rename(&tmp, current.join("theme")).unwrap();
    }

    #[test]
    fn symlink_flip_fires_callback_once_per_burst() {
        let (_tmp, current, _a, theme_b) = fake_omarchy();
        let (tx, rx) = mpsc::channel();
        let _handle = watch_omarchy_dirs(vec![current.clone()], DEBOUNCE, move || {
            let _ = tx.send(());
        })
        .expect("watcher should start on an existing current dir");

        flip_theme(&current, &theme_b);
        rx.recv_timeout(Duration::from_secs(10))
            .expect("symlink flip should trigger the callback");
        assert!(
            rx.recv_timeout(DEBOUNCE * 4).is_err(),
            "a flip burst must be debounced into exactly one callback"
        );
    }

    #[test]
    fn colors_toml_edit_in_resolved_dir_fires_callback() {
        let (_tmp, current, theme_a, _b) = fake_omarchy();
        let (tx, rx) = mpsc::channel();
        let _handle = watch_omarchy_dirs(vec![current], DEBOUNCE, move || {
            let _ = tx.send(());
        })
        .unwrap();

        fs::write(theme_a.join("colors.toml"), "accent = \"#333333\"\n").unwrap();
        rx.recv_timeout(Duration::from_secs(10))
            .expect("editing colors.toml in the resolved theme dir should trigger the callback");
    }

    #[test]
    fn inner_watch_repins_to_new_theme_after_flip() {
        let (_tmp, current, _a, theme_b) = fake_omarchy();
        let (tx, rx) = mpsc::channel();
        let _handle = watch_omarchy_dirs(vec![current.clone()], DEBOUNCE, move || {
            let _ = tx.send(());
        })
        .unwrap();

        flip_theme(&current, &theme_b);
        rx.recv_timeout(Duration::from_secs(10))
            .expect("flip should trigger the callback");
        while rx.recv_timeout(DEBOUNCE * 4).is_ok() {}

        // theme_b was not watched at construction; only the post-flip re-pin
        // can deliver this edit.
        fs::write(theme_b.join("colors.toml"), "accent = \"#444444\"\n").unwrap();
        rx.recv_timeout(Duration::from_secs(10))
            .expect("edit in the newly resolved theme dir should trigger the callback");
    }

    #[test]
    fn missing_current_dirs_yield_no_watcher() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        assert!(watch_omarchy_dirs(vec![missing], DEBOUNCE, || {}).is_none());
        assert!(watch_omarchy_dirs(vec![], DEBOUNCE, || {}).is_none());
    }
}
