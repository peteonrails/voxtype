//! Omarchy theme integration.
//!
//! On startup, both OSD frontends read the active Omarchy theme and map it
//! to a [`Palette`] used by the renderer. The active theme usually lives at
//! `~/.local/state/omarchy/current/theme/colors.toml`, with older installs
//! using `~/.config/omarchy/current/theme/colors.toml`. The colors file has a
//! flat structure: `background`, `foreground`, `accent`, plus the ANSI palette,
//! either numbered (`color0`..=`color15`) or, in current Omarchy themes,
//! named (`red`, `green`, `yellow`, ...).
//!
//! Mapping:
//!
//! - `accent` → waveform fill
//! - `background` → window background (alpha kept from fallback)
//! - `foreground` → held-peak tick
//! - `color2` or `green` (ANSI green) → meter low zone
//! - `color3` or `yellow` (ANSI yellow) → meter mid zone
//! - `color1` or `red` (ANSI red) → meter high zone
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
    Some(theme_dirs_under(Path::new(&home)))
}

/// [`omarchy_theme_dirs`] for a given home directory. Tests use this rather
/// than setting `HOME`, which is process-wide and races with every other
/// test that reads it.
fn theme_dirs_under(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".local/state/omarchy/current/theme"),
        home.join(".config/omarchy/current/theme"),
    ]
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
    red: Option<String>,
    green: Option<String>,
    yellow: Option<String>,
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
    match omarchy_theme_dirs() {
        Some(dirs) => palette_from_dirs(&dirs),
        None => Palette::fallback(),
    }
}

/// [`load_palette`] from the given candidate theme directories.
fn palette_from_dirs(dirs: &[PathBuf]) -> Palette {
    for path in dirs.iter().map(|dir| dir.join("colors.toml")) {
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
        meter_low: ansi(&c.color2, &c.green).unwrap_or(fb.meter_low),
        meter_mid: ansi(&c.color3, &c.yellow).unwrap_or(fb.meter_mid),
        meter_high: ansi(&c.color1, &c.red).unwrap_or(fb.meter_high),
        foreground: c
            .foreground
            .as_deref()
            .and_then(parse_hex)
            .unwrap_or(fb.foreground),
    }
}

/// An ANSI color from its numbered key (older themes) or, when that is
/// absent or invalid, its named key (current Omarchy `colors.toml`).
fn ansi(numbered: &Option<String>, named: &Option<String>) -> Option<Color> {
    numbered
        .as_deref()
        .and_then(parse_hex)
        .or_else(|| named.as_deref().and_then(parse_hex))
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
    /// The watcher of this generation lost its event loop.
    Died(u64),
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
/// A theme switch looks different across Omarchy versions: `theme` may be a
/// symlink that gets flipped, or a real directory that is removed and
/// recreated (with the sibling `theme.name` file rewritten alongside).
/// In-place edits to theme files are a third case. The non-recursive watch
/// on each "current" parent catches the flip, the directory replacement,
/// and the `theme.name` rewrite; a watch on the resolved theme directory
/// catches in-place edits and is re-established from scratch after every
/// change, since a replaced directory silently kills its inotify watch even
/// though the path is unchanged.
///
/// Bursts of events are debounced: `on_change` fires once per quiet period
/// of `debounce`. Returns `None` when none of `current_dirs` exist or no
/// watch could be established; errors never panic.
///
/// notify's inotify event loop panics on any `epoll_wait` error but EINTR.
/// A signal handler that clobbered errno once turned EINTR into EAGAIN (fixed
/// in signal-hook-registry 1.4.8), and other errors remain possible. Every
/// later call on that watcher then panics,
/// and dropping it panics again, which used to abort the whole process. So
/// the watcher is treated as disposable: its death is noticed (the event
/// handler it owns reports when it is dropped), every call into it is
/// guarded, and a fresh watcher replaces it. After a replacement,
/// `on_change` fires only if the theme changed while the watcher was dead,
/// judged against a fingerprint of the theme files as of the last callback;
/// a dead watcher that missed nothing produces no spurious reload.
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
    let mut generation = 0u64;
    let mut armed = Some(arm(&tx, generation, &parents)?);
    let thread_tx = tx.clone();
    let mut delivered = theme_fingerprint(&parents);

    let thread = thread::spawn(move || loop {
        let (mut saw_fs, mut died) = match rx.recv() {
            Ok(WatchMsg::Stop) | Err(_) => {
                discard(armed.take());
                break;
            }
            Ok(WatchMsg::Fs) => (true, false),
            Ok(WatchMsg::Died(g)) if g == generation => (false, true),
            Ok(WatchMsg::Died(_)) => continue,
        };
        loop {
            match rx.recv_timeout(debounce) {
                Ok(WatchMsg::Fs) => saw_fs = true,
                Ok(WatchMsg::Died(g)) => died |= g == generation,
                Ok(WatchMsg::Stop) => {
                    discard(armed.take());
                    return;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    discard(armed.take());
                    return;
                }
            }
        }
        if !died {
            if let Some(a) = armed.as_mut() {
                match watch_theme_targets(&mut a.watcher, &parents, &a.resolved) {
                    Some(resolved) => a.resolved = resolved,
                    None => died = true,
                }
            }
        }
        if died || armed.is_none() {
            discard(armed.take());
            generation += 1;
            armed = arm(&thread_tx, generation, &parents);
            if armed.is_some() {
                tracing::warn!("Omarchy theme watcher lost its event loop; re-armed");
            } else {
                // The failed attempt's own watcher reports Died for this
                // generation when it is dropped; move past it so that does
                // not start another attempt every debounce period.
                generation += 1;
                tracing::warn!(
                    "Omarchy theme watcher lost its event loop and could not be re-armed"
                );
            }
        }
        // Events always report. A replacement reports only what the dead
        // watcher may have missed.
        let now = theme_fingerprint(&parents);
        if saw_fs || now != delivered {
            on_change();
        }
        delivered = now;
    });

    Some(ThemeWatchHandle {
        tx,
        thread: Some(thread),
    })
}

/// What the theme looks like on disk: the directory each "current" parent
/// resolves to, `theme.name` beside it, and every entry in the resolved
/// directory with its modification time, size and, for small files, a hash
/// of the contents (so an edit within one timestamp tick still counts).
/// Cheap: a theme is a few dozen small files.
fn theme_fingerprint(
    parents: &[PathBuf],
) -> Vec<(PathBuf, Option<std::time::SystemTime>, u64, u64)> {
    use std::hash::{Hash, Hasher};
    let stamp = |path: PathBuf| {
        let meta = fs::metadata(&path).ok();
        let modified = meta.as_ref().and_then(|m| m.modified().ok());
        let len = meta.as_ref().map_or(0, |m| m.len());
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        if meta.is_some_and(|m| m.is_file() && m.len() <= 64 * 1024) {
            fs::read(&path).unwrap_or_default().hash(&mut hasher);
        }
        (path, modified, len, hasher.finish())
    };
    let mut out = Vec::new();
    for parent in parents {
        out.push(stamp(parent.join("theme.name")));
        let Ok(dir) = fs::canonicalize(parent.join("theme")) else {
            continue;
        };
        out.push((dir.clone(), None, 0, 0));
        let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
            .map(|it| it.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        entries.sort();
        out.extend(entries.into_iter().map(stamp));
    }
    out
}

/// A watcher and the theme directories it is pinned to.
struct Armed {
    watcher: RecommendedWatcher,
    resolved: Vec<PathBuf>,
}

/// Owned by the watcher's event handler, which notify drops when its event
/// loop thread ends, including when that thread panics.
struct DeathNotice {
    tx: mpsc::Sender<WatchMsg>,
    generation: u64,
}

impl Drop for DeathNotice {
    fn drop(&mut self) {
        let _ = self.tx.send(WatchMsg::Died(self.generation));
    }
}

/// Create a watcher on `parents` and pin it to their resolved theme dirs.
/// `None` when it cannot be created, watches none of `parents`, or dies on
/// the way.
fn arm(tx: &mpsc::Sender<WatchMsg>, generation: u64, parents: &[PathBuf]) -> Option<Armed> {
    let notice = DeathNotice {
        tx: tx.clone(),
        generation,
    };
    let mut watcher =
        match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if res.is_ok() {
                let _ = notice.tx.send(WatchMsg::Fs);
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "failed to create Omarchy theme watcher");
                return None;
            }
        };

    let mut watching = 0usize;
    for parent in parents {
        match guarded(|| watcher.watch(parent, RecursiveMode::NonRecursive)) {
            Some(Ok(())) => watching += 1,
            Some(Err(e)) => {
                tracing::warn!(dir = %parent.display(), error = %e, "failed to watch Omarchy dir")
            }
            None => {
                discard(Some(watcher));
                return None;
            }
        }
    }
    if watching == 0 {
        discard(Some(watcher));
        return None;
    }
    match watch_theme_targets(&mut watcher, parents, &[]) {
        Some(resolved) => Some(Armed { watcher, resolved }),
        None => {
            discard(Some(watcher));
            None
        }
    }
}

/// Run a call into the watcher, `None` when it panics because the watcher's
/// event loop has died.
fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// Drop a watcher. A watcher whose event loop has died panics in `Drop`;
/// contained here, that panic cannot abort the process.
fn discard(armed: Option<impl Sized>) {
    let _ = guarded(move || drop(armed));
}

/// (Re-)pin the watcher to the currently resolved theme directories.
///
/// Old watches are dropped and fresh ones established unconditionally: an
/// unchanged path proves nothing, because a theme switch that removes and
/// recreates the directory takes the inotify watch with it. Unwatch errors
/// (e.g. a watch already dead) are expected and ignored. `None` when the
/// watcher's event loop has died.
fn watch_theme_targets(
    watcher: &mut RecommendedWatcher,
    parents: &[PathBuf],
    old: &[PathBuf],
) -> Option<Vec<PathBuf>> {
    for stale in old {
        let _ = guarded(|| watcher.unwatch(stale))?;
    }
    let resolved: Vec<PathBuf> = parents
        .iter()
        .filter_map(|p| fs::canonicalize(p.join("theme")).ok())
        .collect();
    for dir in &resolved {
        if let Err(e) = guarded(|| watcher.watch(dir, RecursiveMode::NonRecursive))? {
            tracing::debug!(dir = %dir.display(), error = %e, "failed to watch theme dir");
        }
    }
    Some(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    // These never set HOME: it is process-wide, and a test that changed it
    // raced with every other test reading it (watcher_uses_loaded_palette
    // compared the real theme against the fallback when HOME moved between
    // its two reads).

    #[test]
    fn theme_dir_resolves_under_home() {
        let dirs = theme_dirs_under(Path::new("/tmp/fakehome"));
        assert_eq!(
            dirs[0],
            Path::new("/tmp/fakehome/.local/state/omarchy/current/theme")
        );
    }

    #[test]
    fn theme_dirs_include_legacy_config_fallback() {
        let dirs = theme_dirs_under(Path::new("/tmp/fakehome"));
        assert!(dirs[0].ends_with(".local/state/omarchy/current/theme"));
        assert!(dirs[1].ends_with(".config/omarchy/current/theme"));
    }

    #[test]
    fn missing_theme_dir_yields_fallback() {
        let home = tempfile::tempdir().unwrap();
        let dirs = theme_dirs_under(&home.path().join("does-not-exist"));
        assert_eq!(palette_from_dirs(&dirs), Palette::fallback());
    }

    #[test]
    fn a_theme_under_the_given_home_is_loaded() {
        let home = tempfile::tempdir().unwrap();
        let dirs = theme_dirs_under(home.path());
        // Only the legacy location exists: the newer one is skipped.
        fs::create_dir_all(&dirs[1]).unwrap();
        fs::write(dirs[1].join("colors.toml"), "accent = \"#6E89C2\"\n").unwrap();
        let p = palette_from_dirs(&dirs);
        assert!((p.accent.r - 0x6E as f32 / 255.0).abs() < 1e-6);
        assert_eq!(p.background, Palette::fallback().background);
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
    fn palette_from_named_ansi_keys() {
        // Current Omarchy themes name the ANSI colors instead of numbering
        // them (catppuccin's colors.toml).
        let toml_src = r##"
            accent = "#89b4fa"
            background = "#1e1e2e"
            foreground = "#cdd6f4"
            red = "#f38ba8"
            yellow = "#f9e2af"
            green = "#a6e3a1"
        "##;
        let c: OmarchyColors = toml::from_str(toml_src).unwrap();
        let p = palette_from(c);
        assert_eq!(p.meter_high, parse_hex("#f38ba8").unwrap());
        assert_eq!(p.meter_mid, parse_hex("#f9e2af").unwrap());
        assert_eq!(p.meter_low, parse_hex("#a6e3a1").unwrap());
    }

    #[test]
    fn numbered_ansi_keys_win_over_named() {
        let toml_src = r##"
            color1 = "#A48364"
            red = "#f38ba8"
        "##;
        let c: OmarchyColors = toml::from_str(toml_src).unwrap();
        assert_eq!(palette_from(c).meter_high, parse_hex("#A48364").unwrap());
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
    fn watcher_returns_the_palette_it_holds() {
        // ThemeWatcher::new() reads the live HOME; comparing that against a
        // second read raced with tests elsewhere that set HOME. What matters
        // is that the watcher hands back what it was given.
        let mut palette = Palette::fallback();
        palette.accent = Color::rgb(0.1, 0.2, 0.3);
        let w = ThemeWatcher { palette };
        assert_eq!(w.palette(), palette);
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

    /// Fake Omarchy layout for installs where `current/theme` is a REAL
    /// directory (no symlink) and switches replace it in place, rewriting
    /// the sibling `theme.name` file.
    fn fake_omarchy_real_dir() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().join("current");
        let theme = current.join("theme");
        fs::create_dir_all(&theme).unwrap();
        fs::write(theme.join("colors.toml"), "accent = \"#111111\"\n").unwrap();
        fs::write(current.join("theme.name"), "red\n").unwrap();
        (tmp, current, theme)
    }

    #[test]
    fn directory_replacement_fires_once_and_repins_dead_watch() {
        let (_tmp, current, theme) = fake_omarchy_real_dir();
        let (tx, rx) = mpsc::channel();
        let _handle = watch_omarchy_dirs(vec![current.clone()], DEBOUNCE, move || {
            let _ = tx.send(());
        })
        .unwrap();

        // Switch mechanics on real-directory installs: remove and recreate
        // the theme dir, rewrite theme.name.
        fs::remove_dir_all(&theme).unwrap();
        fs::create_dir_all(&theme).unwrap();
        fs::write(theme.join("colors.toml"), "accent = \"#222222\"\n").unwrap();
        fs::write(current.join("theme.name"), "blue\n").unwrap();

        rx.recv_timeout(Duration::from_secs(10))
            .expect("directory replacement should trigger the callback");
        assert!(
            rx.recv_timeout(DEBOUNCE * 4).is_err(),
            "a replacement burst must be debounced into exactly one callback"
        );

        // Replacing the directory killed the original inner inotify watch
        // even though the path is unchanged; only an unconditional re-pin
        // can deliver this in-place edit.
        fs::write(theme.join("colors.toml"), "accent = \"#333333\"\n").unwrap();
        rx.recv_timeout(Duration::from_secs(10))
            .expect("edit inside the recreated theme dir should trigger the callback");
    }

    #[test]
    fn theme_name_rewrite_alone_fires_callback() {
        let (_tmp, current, _theme) = fake_omarchy_real_dir();
        let (tx, rx) = mpsc::channel();
        let _handle = watch_omarchy_dirs(vec![current.clone()], DEBOUNCE, move || {
            let _ = tx.send(());
        })
        .unwrap();

        fs::write(current.join("theme.name"), "blue\n").unwrap();
        rx.recv_timeout(Duration::from_secs(10))
            .expect("rewriting theme.name beside the theme dir should trigger the callback");
    }

    // Linux only: it asserts that nothing fires for a few debounce intervals
    // after re-arming, and macOS's FSEvents can deliver this test's own setup
    // writes late, inside that window. The watcher follows Omarchy themes,
    // which exist only on Linux.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_watcher_that_loses_its_event_loop_is_replaced() {
        let (_tmp, current, theme_a, _b) = fake_omarchy();
        let (tx, rx) = mpsc::channel();
        let handle = watch_omarchy_dirs(vec![current], DEBOUNCE, move || {
            let _ = tx.send(());
        })
        .unwrap();

        // What the first watcher's event handler reports when notify's
        // event loop thread dies. Nothing changed meanwhile, so the
        // replacement reports nothing: a dead loop must not look like a
        // theme switch.
        handle.tx.send(WatchMsg::Died(0)).unwrap();
        assert!(
            rx.recv_timeout(DEBOUNCE * 4).is_err(),
            "re-arming over an unchanged theme must not call back"
        );

        // The first watcher is gone; only the replacement can see this.
        fs::write(theme_a.join("colors.toml"), "accent = \"#555555\"\n").unwrap();
        rx.recv_timeout(Duration::from_secs(10))
            .expect("the replacement watcher should see edits");
        while rx.recv_timeout(DEBOUNCE * 4).is_ok() {}

        // A late notice from the replaced watcher must not re-arm again.
        handle.tx.send(WatchMsg::Died(0)).unwrap();
        assert!(
            rx.recv_timeout(DEBOUNCE * 4).is_err(),
            "a stale death notice must be ignored"
        );
    }

    #[test]
    fn the_fingerprint_moves_with_every_kind_of_theme_change() {
        let (_tmp, current, theme_a, theme_b) = fake_omarchy();
        let parents = vec![current.clone()];
        let before = theme_fingerprint(&parents);
        assert_eq!(theme_fingerprint(&parents), before, "stable when idle");

        fs::write(theme_a.join("colors.toml"), "accent = \"#999999\"\n").unwrap();
        let edited = theme_fingerprint(&parents);
        assert_ne!(edited, before, "an in-place edit");

        flip_theme(&current, &theme_b);
        let flipped = theme_fingerprint(&parents);
        assert_ne!(flipped, edited, "a symlink flip");

        fs::write(current.join("theme.name"), "b\n").unwrap();
        assert_ne!(theme_fingerprint(&parents), flipped, "a theme.name rewrite");
    }

    #[test]
    fn discarding_a_dead_watcher_contains_its_drop_panic() {
        // notify's INotifyWatcher panics in Drop once its event loop is
        // gone. Dropped while the thread unwinds, that aborted the process.
        struct PanicsOnDrop;
        impl Drop for PanicsOnDrop {
            fn drop(&mut self) {
                panic!("event loop is gone");
            }
        }
        discard(Some(PanicsOnDrop));
        assert_eq!(guarded(|| -> u8 { panic!("event loop is gone") }), None);
        assert_eq!(guarded(|| 7), Some(7));
    }

    #[test]
    fn missing_current_dirs_yield_no_watcher() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        assert!(watch_omarchy_dirs(vec![missing], DEBOUNCE, || {}).is_none());
        assert!(watch_omarchy_dirs(vec![], DEBOUNCE, || {}).is_none());
    }
}
