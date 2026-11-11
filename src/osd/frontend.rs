//! Resolve `[osd] frontend` to the frontend that actually runs.
//!
//! `auto` (the default) picks the best frontend this system can run:
//!
//! 1. `quickshell` when the `voxtype-osd-quickshell` launcher resolves
//!    (beside the running executable, then on `PATH`), `qs` or `quickshell`
//!    is on `PATH`, and a Wayland session is present;
//! 2. else `gtk4` when `voxtype-osd-gtk4` resolves, its shared libraries
//!    load (gtk4-layer-shell is missing on some distros), and a display is
//!    present;
//! 3. else `native` when `voxtype-osd-native` resolves;
//! 4. else none.
//!
//! An explicit `gtk4`, `native` or `quickshell` keeps the `voxtype-osd`
//! launcher's long-standing behavior: the configured binary when installed,
//! otherwise the next installed one in a fixed fallback order, with no
//! display checks.
//!
//! All system access goes through [`OsdProbe`] so every branch is testable
//! with a fake; [`SystemProbe`] is the real one.

use std::path::{Path, PathBuf};

use super::config::OsdFrontend;

/// Environment variable that overrides `[osd] frontend`, read by the
/// daemon and by the `voxtype-osd` launcher.
pub const FRONTEND_ENV: &str = "VOXTYPE_OSD_FRONTEND";

/// What the resolver needs to know about the system.
pub trait OsdProbe {
    /// Locate one of voxtype's own OSD binaries: beside the running
    /// executable first, then on `PATH`.
    fn find_voxtype_binary(&self, name: &str) -> Option<PathBuf>;
    /// Whether a third-party program (`qs`, `quickshell`) is on `PATH`.
    fn on_path(&self, name: &str) -> bool;
    /// Whether a Wayland compositor is reachable.
    fn wayland_session(&self) -> bool;
    /// Whether any display (Wayland or X11) is reachable.
    fn display_present(&self) -> bool;
    /// Shared libraries `binary` needs that the dynamic loader can't find.
    /// Empty when everything loads or when that can't be determined.
    fn missing_libraries(&self, binary: &Path) -> Vec<String>;
}

/// Libraries the GTK4 OSD needs that some distros don't package (Ubuntu
/// 24.04 has no gtk4-layer-shell), so the binary exists but exits 127.
const GTK4_LIBRARIES: [&str; 2] = ["libgtk4-layer-shell.so.0", "libgtk-4.so.1"];

/// Probes the real system.
pub struct SystemProbe;

impl OsdProbe for SystemProbe {
    fn find_voxtype_binary(&self, name: &str) -> Option<PathBuf> {
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
        {
            let sibling = dir.join(name);
            if sibling.is_file() {
                return Some(sibling);
            }
        }
        which::which(name).ok()
    }

    fn on_path(&self, name: &str) -> bool {
        which::which(name).is_ok()
    }

    fn wayland_session(&self) -> bool {
        wayland_reachable(
            std::env::var_os("WAYLAND_DISPLAY").as_deref(),
            std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
        )
    }

    fn display_present(&self) -> bool {
        self.wayland_session() || std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty())
    }

    /// Asks `ldd`, which resolves the binary's own RPATH/RUNPATH the way
    /// the loader will. Without `ldd`, checks the GTK4 libraries against
    /// `ldconfig -p`. When neither runs, assumes everything loads, which
    /// is how gtk4 was treated before this check existed.
    fn missing_libraries(&self, binary: &Path) -> Vec<String> {
        let run = |cmd: &str, args: &[&std::ffi::OsStr]| {
            std::process::Command::new(cmd)
                .args(args)
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        };
        if let Some(out) = run("ldd", &[binary.as_os_str()]) {
            return parse_ldd_missing(&out);
        }
        for ldconfig in ["ldconfig", "/sbin/ldconfig", "/usr/sbin/ldconfig"] {
            if let Some(out) = run(ldconfig, &["-p".as_ref()]) {
                return GTK4_LIBRARIES
                    .iter()
                    .filter(|lib| !ldconfig_has(&out, lib))
                    .map(|lib| lib.to_string())
                    .collect();
            }
        }
        Vec::new()
    }
}

/// Sonames `ldd` reports as `=> not found`.
fn parse_ldd_missing(output: &str) -> Vec<String> {
    output
        .lines()
        .filter(|l| l.contains("=> not found"))
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

/// Whether `ldconfig -p` lists `soname`.
fn ldconfig_has(output: &str, soname: &str) -> bool {
    output
        .lines()
        .any(|l| l.split_whitespace().next() == Some(soname))
}

/// Why an installed GTK4 OSD binary can't load, or `None` when it can.
fn gtk4_library_blocker(missing: &[String]) -> Option<String> {
    let lib = missing
        .iter()
        .find(|m| m.starts_with("libgtk4-layer-shell"))
        .or_else(|| missing.first())?;
    let name = lib.split(".so").next().unwrap_or(lib);
    Some(format!("gtk4 OSD installed but {name} is missing"))
}

/// `WAYLAND_DISPLAY` set, or the default `wayland-0` socket present in
/// the runtime dir. The second case covers a daemon started by systemd
/// before the compositor imported its environment: libwayland clients
/// connect to `wayland-0` when `WAYLAND_DISPLAY` is unset.
fn wayland_reachable(
    wayland_display: Option<&std::ffi::OsStr>,
    runtime_dir: Option<&std::ffi::OsStr>,
) -> bool {
    if wayland_display.is_some_and(|d| !d.is_empty()) {
        return true;
    }
    runtime_dir
        .filter(|d| !d.is_empty())
        .is_some_and(|d| Path::new(d).join("wayland-0").exists())
}

/// The frontend a configured value resolves to, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The value the resolution started from.
    pub configured: OsdFrontend,
    /// The frontend that runs, or `None` when nothing can.
    pub active: Option<OsdFrontend>,
    /// Path of the active frontend's binary.
    pub path: Option<PathBuf>,
    /// One line for logs and `setup osd --status`.
    pub reason: String,
}

impl Resolution {
    /// `quickshell`, `gtk4`, `native`, or `none`.
    pub fn active_name(&self) -> &'static str {
        self.active.map_or("none", OsdFrontend::as_str)
    }
}

/// Names the Quickshell program is installed under, in lookup order. Some
/// packages ship only one of them.
pub const QUICKSHELL_PROGRAMS: [&str; 2] = ["qs", "quickshell"];

/// The first Quickshell program `lookup` resolves, in
/// [`QUICKSHELL_PROGRAMS`] order. The `voxtype-osd-quickshell` launcher
/// execs this, so it runs whatever the resolver counted as available.
pub fn find_quickshell_program(lookup: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    QUICKSHELL_PROGRAMS.iter().find_map(|name| lookup(name))
}

/// Why the Quickshell OSD can't run here, or `None` when it can.
fn quickshell_blocker(probe: &dyn OsdProbe) -> Option<String> {
    if probe
        .find_voxtype_binary(OsdFrontend::Quickshell.binary_name())
        .is_none()
    {
        return Some("voxtype-osd-quickshell is not installed".into());
    }
    if !QUICKSHELL_PROGRAMS.iter().any(|p| probe.on_path(p)) {
        return Some("Quickshell (qs) is not on PATH".into());
    }
    if !probe.wayland_session() {
        return Some("no Wayland session".into());
    }
    None
}

/// True when `auto` would pick the Quickshell OSD on this system.
pub fn quickshell_available(probe: &dyn OsdProbe) -> bool {
    quickshell_blocker(probe).is_none()
}

/// Resolve `configured` against what this system can run.
pub fn resolve(configured: OsdFrontend, probe: &dyn OsdProbe) -> Resolution {
    match configured {
        OsdFrontend::Auto => resolve_auto(probe),
        explicit => resolve_explicit(explicit, probe),
    }
}

fn resolve_auto(probe: &dyn OsdProbe) -> Resolution {
    let found = |f: OsdFrontend| probe.find_voxtype_binary(f.binary_name());
    let done = |active: Option<OsdFrontend>, path: Option<PathBuf>, reason: String| Resolution {
        configured: OsdFrontend::Auto,
        active,
        path,
        reason: format!("auto: {reason}"),
    };

    let qs_blocker = match quickshell_blocker(probe) {
        None => {
            return done(
                Some(OsdFrontend::Quickshell),
                found(OsdFrontend::Quickshell),
                "Quickshell and a Wayland session are available".into(),
            );
        }
        Some(why) => why,
    };

    let gtk_blocker = match found(OsdFrontend::Gtk4) {
        None => "voxtype-osd-gtk4 is not installed".to_string(),
        Some(path) => match gtk4_library_blocker(&probe.missing_libraries(&path)) {
            Some(why) => why,
            None if probe.display_present() => {
                return done(
                    Some(OsdFrontend::Gtk4),
                    Some(path),
                    format!("{qs_blocker}, using gtk4"),
                );
            }
            None => "no display".to_string(),
        },
    };

    match found(OsdFrontend::Native) {
        Some(path) => done(
            Some(OsdFrontend::Native),
            Some(path),
            format!("{qs_blocker}; {gtk_blocker}; using native"),
        ),
        None => done(
            None,
            None,
            format!("{qs_blocker}; {gtk_blocker}; voxtype-osd-native is not installed; no OSD"),
        ),
    }
}

/// The `voxtype-osd` launcher's fallback order for an explicit choice.
fn fallback_chain(preferred: OsdFrontend) -> [OsdFrontend; 3] {
    use OsdFrontend::*;
    match preferred {
        Native => [Native, Gtk4, Quickshell],
        Quickshell => [Quickshell, Gtk4, Native],
        Gtk4 | Auto => [Gtk4, Native, Quickshell],
    }
}

fn resolve_explicit(configured: OsdFrontend, probe: &dyn OsdProbe) -> Resolution {
    let name = configured.as_str();
    let hit = fallback_chain(configured).into_iter().find_map(|f| {
        probe
            .find_voxtype_binary(f.binary_name())
            .map(|path| (f, path))
    });
    let (active, path, reason) = match hit {
        Some((f, path)) if f == configured => (Some(f), Some(path), format!("configured {name}")),
        Some((f, path)) => (
            Some(f),
            Some(path),
            format!(
                "configured {name}, but {} is not installed; using {}",
                configured.binary_name(),
                f.as_str()
            ),
        ),
        None => (
            None,
            None,
            format!("configured {name}, but no OSD frontend is installed"),
        ),
    };
    Resolution {
        configured,
        active,
        path,
        reason,
    }
}

/// The configured frontend after the `VOXTYPE_OSD_FRONTEND` override,
/// matching the precedence the `voxtype-osd` launcher applies.
pub fn effective_configured(config_value: OsdFrontend) -> OsdFrontend {
    std::env::var(FRONTEND_ENV)
        .ok()
        .and_then(|v| OsdFrontend::parse_str(&v))
        .unwrap_or(config_value)
}

// ---------------------------------------------------------------- daemon

/// File in the runtime dir where the daemon records the frontend it runs.
pub fn active_file_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("osd_frontend")
}

/// Record the daemon's active frontend (`quickshell`, `gtk4`, `native` or
/// `none`). Best effort: status falls back to resolving locally.
pub fn publish_active(runtime_dir: &Path, active: &str) {
    let _ = std::fs::create_dir_all(runtime_dir);
    let _ = std::fs::write(active_file_path(runtime_dir), active);
}

/// The frontend a running daemon published, if it is one of the known
/// names. The caller checks that the daemon is alive.
pub fn read_published(runtime_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(active_file_path(runtime_dir)).ok()?;
    let name = raw.trim();
    matches!(name, "quickshell" | "gtk4" | "native" | "none").then(|| name.to_string())
}

// ----------------------------------------------------------- offer state

/// `~/.local/state/voxtype/osd-offer.json`: whether the user dismissed the
/// offer to switch to the Quickshell OSD.
pub fn offer_state_path(state_dir: &Path) -> PathBuf {
    state_dir.join("osd-offer.json")
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct OfferState {
    #[serde(default)]
    dismissed: bool,
}

/// True when the offer was dismissed. A missing or unreadable file reads
/// as not dismissed.
pub fn offer_dismissed(state_dir: &Path) -> bool {
    std::fs::read_to_string(offer_state_path(state_dir))
        .ok()
        .and_then(|s| serde_json::from_str::<OfferState>(&s).ok())
        .is_some_and(|s| s.dismissed)
}

/// Record or clear the dismissal. Clearing removes the file.
pub fn set_offer_dismissed(state_dir: &Path, dismissed: bool) -> std::io::Result<()> {
    let path = offer_state_path(state_dir);
    if !dismissed {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(state_dir)?;
    let body = serde_json::to_string(&OfferState { dismissed: true })?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body + "\n")?;
    std::fs::rename(&tmp, &path)
}

/// `voxtype setup osd --status` (contract C3). Field order is the JSON key
/// order.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OsdStatus {
    pub configured: &'static str,
    pub active: &'static str,
    pub reason: String,
    pub quickshell_available: bool,
    pub offer: bool,
    pub offer_dismissed: bool,
}

impl OsdStatus {
    pub fn build(configured: OsdFrontend, probe: &dyn OsdProbe, offer_dismissed: bool) -> Self {
        let resolution = resolve(configured, probe);
        let quickshell_available = quickshell_available(probe);
        let offer = matches!(configured, OsdFrontend::Gtk4 | OsdFrontend::Native)
            && quickshell_available
            && !offer_dismissed;
        Self {
            configured: configured.as_str(),
            active: resolution.active_name(),
            reason: resolution.reason,
            quickshell_available,
            offer,
            offer_dismissed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[derive(Default)]
    struct Fake {
        binaries: HashSet<&'static str>,
        programs: HashSet<&'static str>,
        wayland: bool,
        x11: bool,
        missing_libs: Vec<String>,
    }

    impl Fake {
        fn with(binaries: &[&'static str], programs: &[&'static str], wayland: bool) -> Self {
            Self {
                binaries: binaries.iter().copied().collect(),
                programs: programs.iter().copied().collect(),
                wayland,
                x11: false,
                missing_libs: Vec::new(),
            }
        }
    }

    impl OsdProbe for Fake {
        fn find_voxtype_binary(&self, name: &str) -> Option<PathBuf> {
            self.binaries
                .contains(name)
                .then(|| PathBuf::from("/fake/bin").join(name))
        }
        fn on_path(&self, name: &str) -> bool {
            self.programs.contains(name)
        }
        fn wayland_session(&self) -> bool {
            self.wayland
        }
        fn display_present(&self) -> bool {
            self.wayland || self.x11
        }
        fn missing_libraries(&self, binary: &Path) -> Vec<String> {
            assert_eq!(
                binary,
                Path::new("/fake/bin").join(GTK),
                "only gtk4 is checked"
            );
            self.missing_libs.clone()
        }
    }

    const QS: &str = "voxtype-osd-quickshell";
    const GTK: &str = "voxtype-osd-gtk4";
    const NATIVE: &str = "voxtype-osd-native";
    const ALL: [&str; 3] = [QS, GTK, NATIVE];

    fn auto(probe: &Fake) -> Resolution {
        resolve(OsdFrontend::Auto, probe)
    }

    #[test]
    fn auto_picks_quickshell_when_everything_is_there() {
        let r = auto(&Fake::with(&ALL, &["qs"], true));
        assert_eq!(r.active, Some(OsdFrontend::Quickshell));
        assert_eq!(r.path, Some(PathBuf::from("/fake/bin").join(QS)));
        assert_eq!(
            r.reason,
            "auto: Quickshell and a Wayland session are available"
        );
    }

    #[test]
    fn auto_accepts_the_quickshell_program_name() {
        let r = auto(&Fake::with(&ALL, &["quickshell"], true));
        assert_eq!(r.active, Some(OsdFrontend::Quickshell));
    }

    #[test]
    fn auto_falls_to_gtk4_without_the_launcher() {
        let r = auto(&Fake::with(&[GTK, NATIVE], &["qs"], true));
        assert_eq!(r.active, Some(OsdFrontend::Gtk4));
        assert_eq!(
            r.reason,
            "auto: voxtype-osd-quickshell is not installed, using gtk4"
        );
    }

    #[test]
    fn auto_falls_to_gtk4_without_qs() {
        let r = auto(&Fake::with(&ALL, &[], true));
        assert_eq!(r.active, Some(OsdFrontend::Gtk4));
        assert_eq!(r.reason, "auto: Quickshell (qs) is not on PATH, using gtk4");
    }

    #[test]
    fn auto_falls_to_gtk4_on_x11() {
        let mut fake = Fake::with(&ALL, &["qs"], false);
        fake.x11 = true;
        let r = auto(&fake);
        assert_eq!(r.active, Some(OsdFrontend::Gtk4));
        assert_eq!(r.reason, "auto: no Wayland session, using gtk4");
    }

    #[test]
    fn auto_falls_to_native_without_gtk4() {
        let r = auto(&Fake::with(&[NATIVE], &[], true));
        assert_eq!(r.active, Some(OsdFrontend::Native));
        assert_eq!(
            r.reason,
            "auto: voxtype-osd-quickshell is not installed; \
             voxtype-osd-gtk4 is not installed; using native"
        );
    }

    #[test]
    fn auto_falls_to_native_without_a_display() {
        let r = auto(&Fake::with(&ALL, &["qs"], false));
        assert_eq!(r.active, Some(OsdFrontend::Native));
        assert_eq!(
            r.reason,
            "auto: no Wayland session; no display; using native"
        );
    }

    #[test]
    fn auto_skips_gtk4_whose_libraries_do_not_load() {
        // Ubuntu 24.04: voxtype-osd-gtk4 is installed, gtk4-layer-shell isn't.
        let mut fake = Fake::with(&[GTK, NATIVE], &[], true);
        fake.missing_libs = vec!["libgtk4-layer-shell.so.0".into()];
        let r = auto(&fake);
        assert_eq!(r.active, Some(OsdFrontend::Native));
        assert_eq!(
            r.reason,
            "auto: voxtype-osd-quickshell is not installed; \
             gtk4 OSD installed but libgtk4-layer-shell is missing; using native"
        );

        // Any other unresolved library blocks it too, and is named.
        fake.missing_libs = vec!["libgtk-4.so.1".into()];
        let r = auto(&fake);
        assert_eq!(r.active, Some(OsdFrontend::Native));
        assert!(
            r.reason
                .contains("gtk4 OSD installed but libgtk-4 is missing"),
            "{}",
            r.reason
        );

        // Without native, nothing runs.
        let mut fake = Fake::with(&[GTK], &[], true);
        fake.missing_libs = vec!["libgtk4-layer-shell.so.0".into()];
        assert_eq!(auto(&fake).active, None);
    }

    #[test]
    fn explicit_gtk4_is_not_library_checked() {
        let mut fake = Fake::with(&[GTK, NATIVE], &[], true);
        fake.missing_libs = vec!["libgtk4-layer-shell.so.0".into()];
        assert_eq!(
            resolve(OsdFrontend::Gtk4, &fake).active,
            Some(OsdFrontend::Gtk4)
        );
    }

    #[test]
    fn ldd_and_ldconfig_output_parsing() {
        let ldd = "\tlinux-vdso.so.1 (0x00007ffc)\n\
                   \tlibgtk4-layer-shell.so.0 => not found\n\
                   \tlibgtk-4.so.1 => /usr/lib/libgtk-4.so.1 (0x00007f)\n\
                   \tlibfoo.so.2 => not found\n";
        assert_eq!(
            parse_ldd_missing(ldd),
            vec!["libgtk4-layer-shell.so.0", "libfoo.so.2"]
        );
        assert!(parse_ldd_missing("\tnot a dynamic executable\n").is_empty());

        let ldconfig = "1234 libs found in cache `/etc/ld.so.cache'\n\
                        \tlibgtk-4.so.1 (libc6,x86-64) => /usr/lib/libgtk-4.so.1\n";
        assert!(ldconfig_has(ldconfig, "libgtk-4.so.1"));
        assert!(!ldconfig_has(ldconfig, "libgtk4-layer-shell.so.0"));
    }

    #[test]
    fn quickshell_program_lookup_prefers_qs_then_quickshell() {
        let on = |names: &'static [&'static str]| {
            move |n: &str| {
                names
                    .contains(&n)
                    .then(|| PathBuf::from("/usr/bin").join(n))
            }
        };
        assert_eq!(
            find_quickshell_program(on(&["qs", "quickshell"])),
            Some(PathBuf::from("/usr/bin/qs"))
        );
        assert_eq!(
            find_quickshell_program(on(&["quickshell"])),
            Some(PathBuf::from("/usr/bin/quickshell"))
        );
        assert_eq!(find_quickshell_program(on(&[])), None);
    }

    #[test]
    fn auto_resolves_to_none_when_nothing_is_installed() {
        let r = auto(&Fake::with(&[], &["qs"], true));
        assert_eq!(r.active, None);
        assert_eq!(r.active_name(), "none");
        assert!(r.reason.ends_with("no OSD"), "{}", r.reason);
    }

    #[test]
    fn auto_without_display_and_without_native_is_none() {
        let r = auto(&Fake::with(&[GTK], &[], false));
        assert_eq!(r.active, None);
    }

    #[test]
    fn explicit_values_are_honored_whatever_else_is_available() {
        let fake = Fake::with(&ALL, &["qs"], true);
        for f in [
            OsdFrontend::Gtk4,
            OsdFrontend::Native,
            OsdFrontend::Quickshell,
        ] {
            let r = resolve(f, &fake);
            assert_eq!(r.active, Some(f));
            assert_eq!(r.reason, format!("configured {}", f.as_str()));
        }
    }

    #[test]
    fn explicit_values_skip_display_checks() {
        // The launcher never checked for a display; explicit values keep
        // exactly that behavior.
        let r = resolve(OsdFrontend::Quickshell, &Fake::with(&[QS], &[], false));
        assert_eq!(r.active, Some(OsdFrontend::Quickshell));
    }

    #[test]
    fn explicit_values_fall_back_in_the_launcher_order() {
        let r = resolve(OsdFrontend::Gtk4, &Fake::with(&[NATIVE, QS], &[], true));
        assert_eq!(r.active, Some(OsdFrontend::Native));
        assert_eq!(
            r.reason,
            "configured gtk4, but voxtype-osd-gtk4 is not installed; using native"
        );
        let r = resolve(OsdFrontend::Native, &Fake::with(&[GTK, QS], &[], true));
        assert_eq!(r.active, Some(OsdFrontend::Gtk4));
        let r = resolve(
            OsdFrontend::Quickshell,
            &Fake::with(&[NATIVE, GTK], &[], true),
        );
        assert_eq!(r.active, Some(OsdFrontend::Gtk4));
        let r = resolve(OsdFrontend::Native, &Fake::with(&[], &[], true));
        assert_eq!(r.active, None);
        assert_eq!(
            r.reason,
            "configured native, but no OSD frontend is installed"
        );
    }

    #[test]
    fn wayland_reachable_uses_the_variable_or_the_default_socket() {
        use std::ffi::OsStr;
        assert!(wayland_reachable(Some(OsStr::new("wayland-1")), None));
        assert!(!wayland_reachable(Some(OsStr::new("")), None));
        assert!(!wayland_reachable(None, None));
        let dir = tempfile::tempdir().unwrap();
        assert!(!wayland_reachable(None, Some(dir.path().as_os_str())));
        std::fs::write(dir.path().join("wayland-0"), "").unwrap();
        assert!(wayland_reachable(None, Some(dir.path().as_os_str())));
    }

    #[test]
    fn status_offers_quickshell_to_explicit_gtk4_and_native_only() {
        let fake = Fake::with(&ALL, &["qs"], true);
        assert!(OsdStatus::build(OsdFrontend::Gtk4, &fake, false).offer);
        assert!(OsdStatus::build(OsdFrontend::Native, &fake, false).offer);
        assert!(!OsdStatus::build(OsdFrontend::Auto, &fake, false).offer);
        assert!(!OsdStatus::build(OsdFrontend::Quickshell, &fake, false).offer);
        // Dismissed, or nothing to switch to: no offer.
        assert!(!OsdStatus::build(OsdFrontend::Gtk4, &fake, true).offer);
        let no_qs = Fake::with(&ALL, &[], true);
        assert!(!OsdStatus::build(OsdFrontend::Gtk4, &no_qs, false).offer);
    }

    #[test]
    fn status_json_has_the_contract_key_order() {
        let fake = Fake::with(&ALL, &["qs"], true);
        let s = OsdStatus::build(OsdFrontend::Gtk4, &fake, false);
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"configured":"gtk4","active":"gtk4","reason":"configured gtk4","quickshell_available":true,"offer":true,"offer_dismissed":false}"#
        );
    }

    #[test]
    fn offer_dismissal_set_clear_and_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("voxtype");
        // Missing directory and file: not dismissed, clearing is a no-op.
        assert!(!offer_dismissed(&state));
        set_offer_dismissed(&state, false).unwrap();
        // Set creates the directory and file.
        set_offer_dismissed(&state, true).unwrap();
        assert!(offer_dismissed(&state));
        let body = std::fs::read_to_string(offer_state_path(&state)).unwrap();
        assert_eq!(body.trim(), r#"{"dismissed":true}"#);
        // Setting twice is fine.
        set_offer_dismissed(&state, true).unwrap();
        assert!(offer_dismissed(&state));
        // Clear removes it.
        set_offer_dismissed(&state, false).unwrap();
        assert!(!offer_state_path(&state).exists());
        assert!(!offer_dismissed(&state));
    }

    #[test]
    fn unreadable_offer_file_reads_as_not_dismissed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(offer_state_path(dir.path()), "not json").unwrap();
        assert!(!offer_dismissed(dir.path()));
        std::fs::write(offer_state_path(dir.path()), "{}").unwrap();
        assert!(!offer_dismissed(dir.path()));
    }

    #[test]
    fn published_frontend_round_trips_and_rejects_garbage() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_published(dir.path()), None);
        publish_active(dir.path(), "quickshell");
        assert_eq!(read_published(dir.path()).as_deref(), Some("quickshell"));
        std::fs::write(active_file_path(dir.path()), "bogus").unwrap();
        assert_eq!(read_published(dir.path()), None);
    }
}
