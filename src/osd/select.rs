//! Which OSD frontend `voxtype-osd` launches.
//!
//! Shared by the `voxtype-osd` wrapper, which execs the result, and the
//! daemon's OSD supervisor, which runs the same selection once at startup
//! so it can tell the user with a desktop notification when their chosen
//! frontend isn't available and another one is standing in for it.
//!
//! [`OsdFrontend::Auto`] (the default) prefers Quickshell on Omarchy when
//! `qs` and the `voxtype-osd-quickshell` launcher are both installed:
//! Omarchy's own shell is Quickshell, and the Quickshell OSD follows
//! Omarchy themes live. Everywhere else it prefers GTK4, the frontend
//! every release before `auto` launched, so non-Omarchy installs see no
//! change. An explicit frontend always wins over the detection; it only
//! falls back when its binary (or, for Quickshell, `qs`) is missing.

use std::path::PathBuf;

use crate::osd::config::OsdFrontend;

/// Name of the Quickshell executable the Quickshell launcher execs.
pub const QS_BIN: &str = "qs";

/// What this system offers, probed once per selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Environment {
    /// An Omarchy install is present (its "current theme" directory exists).
    pub omarchy: bool,
    /// Quickshell's `qs` is on PATH.
    pub qs_installed: bool,
}

impl Environment {
    pub fn probe() -> Self {
        Self {
            omarchy: !crate::osd::theme::omarchy_current_dirs().is_empty(),
            qs_installed: which::which(QS_BIN).is_ok(),
        }
    }
}

/// Why the preferred frontend could not be launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// The frontend's binary isn't next to `voxtype-osd` or on PATH.
    NotInstalled,
    /// The Quickshell launcher is installed but `qs` itself is not.
    QuickshellMissing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// What the user asked for (CLI, env, or config), possibly `Auto`.
    pub requested: OsdFrontend,
    /// The concrete frontend `requested` resolves to before fallback.
    pub preferred: OsdFrontend,
    /// The frontend that will actually run, with its binary.
    pub chosen: Option<(OsdFrontend, PathBuf)>,
    /// Why `preferred` isn't the one that runs, if it isn't.
    pub preferred_unavailable: Option<Unavailable>,
}

/// A user-facing message: log line and desktop notification text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// Order in which frontends are tried once `preferred` is known.
/// Quickshell sits at the bottom of non-Quickshell chains because users
/// who didn't ask for it usually don't have `qs` installed.
pub fn fallback_chain(preferred: OsdFrontend) -> [OsdFrontend; 3] {
    use OsdFrontend::*;
    match preferred {
        Auto | Gtk4 => [Gtk4, Native, Quickshell],
        Native => [Native, Gtk4, Quickshell],
        Quickshell => [Quickshell, Gtk4, Native],
    }
}

/// Resolve `requested` against the environment and the installed
/// binaries. `find` maps a binary name to its path, or `None` when it
/// isn't installed.
pub fn select(
    requested: OsdFrontend,
    env: Environment,
    find: impl Fn(&str) -> Option<PathBuf>,
) -> Selection {
    let usable = |f: OsdFrontend| -> Result<PathBuf, Unavailable> {
        let path = find(f.binary_name()).ok_or(Unavailable::NotInstalled)?;
        if f == OsdFrontend::Quickshell && !env.qs_installed {
            return Err(Unavailable::QuickshellMissing);
        }
        Ok(path)
    };

    let preferred = match requested {
        OsdFrontend::Auto if env.omarchy && usable(OsdFrontend::Quickshell).is_ok() => {
            OsdFrontend::Quickshell
        }
        OsdFrontend::Auto => OsdFrontend::Gtk4,
        explicit => explicit,
    };

    let mut chosen = None;
    let mut preferred_unavailable = None;
    for f in fallback_chain(preferred) {
        match usable(f) {
            Ok(path) => {
                chosen = Some((f, path));
                break;
            }
            Err(why) if f == preferred => preferred_unavailable = Some(why),
            Err(_) => {}
        }
    }

    Selection {
        requested,
        preferred,
        chosen,
        preferred_unavailable,
    }
}

impl Selection {
    pub fn chosen_frontend(&self) -> Option<OsdFrontend> {
        self.chosen.as_ref().map(|(f, _)| *f)
    }

    /// The notice to show when the user's explicit choice is being
    /// substituted, or when no frontend can run at all. `None` when the
    /// requested frontend runs, or when `auto` merely settled on its
    /// second choice (nothing the user asked for is being overridden).
    pub fn notice(&self) -> Option<Notice> {
        let Some(chosen) = self.chosen_frontend() else {
            let qs_hint = if self.preferred_unavailable == Some(Unavailable::QuickshellMissing) {
                " Quickshell (qs) is not installed either."
            } else {
                ""
            };
            return Some(Notice {
                title: "Voxtype: no OSD frontend installed".to_string(),
                body: format!(
                    "None of voxtype-osd-gtk4, voxtype-osd-native or voxtype-osd-quickshell \
                     was found next to voxtype-osd or on PATH, so no on-screen display will \
                     show.{qs_hint} Reinstall voxtype with its OSD binaries, or set \
                     [osd] enabled = false to stop trying."
                ),
            });
        };
        if self.requested == OsdFrontend::Auto || chosen == self.requested {
            return None;
        }

        let requested = self.requested.as_str();
        let using = chosen.as_str();
        let why = match self.preferred_unavailable {
            Some(Unavailable::QuickshellMissing) => {
                "Quickshell (qs) is not installed. Install it (Arch: sudo pacman -S quickshell)"
                    .to_string()
            }
            _ => format!(
                "{} was not found next to voxtype-osd or on PATH. Reinstall voxtype with \
                 its OSD binaries{}",
                self.requested.binary_name(),
                source_build_hint(self.requested),
            ),
        };
        Some(Notice {
            title: format!("Voxtype: OSD frontend '{requested}' unavailable"),
            body: format!(
                "Showing the {using} OSD instead. {why}, or set [osd] frontend = \"{using}\" \
                 to keep {using}."
            ),
        })
    }
}

fn source_build_hint(frontend: OsdFrontend) -> &'static str {
    match frontend {
        OsdFrontend::Gtk4 => " (source builds: --features osd-gtk4)",
        OsdFrontend::Native => " (source builds: --features osd-native)",
        OsdFrontend::Quickshell => " (source builds: --bin voxtype-osd-quickshell)",
        OsdFrontend::Auto => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use OsdFrontend::*;

    const OMARCHY_WITH_QS: Environment = Environment {
        omarchy: true,
        qs_installed: true,
    };
    const PLAIN: Environment = Environment {
        omarchy: false,
        qs_installed: false,
    };

    fn installed(names: &'static [&'static str]) -> impl Fn(&str) -> Option<PathBuf> {
        move |name| {
            names
                .contains(&name)
                .then(|| PathBuf::from("/usr/lib/voxtype").join(name))
        }
    }

    const ALL: &[&str] = &[
        "voxtype-osd-gtk4",
        "voxtype-osd-native",
        "voxtype-osd-quickshell",
    ];

    #[test]
    fn auto_picks_quickshell_on_omarchy() {
        let s = select(Auto, OMARCHY_WITH_QS, installed(ALL));
        assert_eq!(s.preferred, Quickshell);
        assert_eq!(s.chosen_frontend(), Some(Quickshell));
        assert_eq!(s.notice(), None);
    }

    #[test]
    fn auto_keeps_gtk4_off_omarchy() {
        let env = Environment {
            omarchy: false,
            qs_installed: true,
        };
        let s = select(Auto, env, installed(ALL));
        assert_eq!(s.chosen_frontend(), Some(Gtk4));
        assert_eq!(s.notice(), None);
    }

    #[test]
    fn auto_on_omarchy_without_qs_quietly_uses_gtk4() {
        let env = Environment {
            omarchy: true,
            qs_installed: false,
        };
        let s = select(Auto, env, installed(ALL));
        assert_eq!(s.preferred, Gtk4);
        assert_eq!(s.chosen_frontend(), Some(Gtk4));
        assert_eq!(s.notice(), None);
    }

    #[test]
    fn auto_on_omarchy_without_quickshell_launcher_uses_gtk4() {
        let s = select(
            Auto,
            OMARCHY_WITH_QS,
            installed(&["voxtype-osd-gtk4", "voxtype-osd-native"]),
        );
        assert_eq!(s.chosen_frontend(), Some(Gtk4));
        assert_eq!(s.notice(), None);
    }

    #[test]
    fn auto_falling_back_from_gtk4_is_quiet() {
        let s = select(Auto, PLAIN, installed(&["voxtype-osd-native"]));
        assert_eq!(s.chosen_frontend(), Some(Native));
        assert_eq!(s.notice(), None);
    }

    #[test]
    fn explicit_choice_wins_over_detection() {
        let s = select(Gtk4, OMARCHY_WITH_QS, installed(ALL));
        assert_eq!(s.chosen_frontend(), Some(Gtk4));
        let s = select(Native, OMARCHY_WITH_QS, installed(ALL));
        assert_eq!(s.chosen_frontend(), Some(Native));
        let s = select(Quickshell, PLAIN.with_qs(), installed(ALL));
        assert_eq!(s.chosen_frontend(), Some(Quickshell));
        assert_eq!(s.notice(), None);
    }

    #[test]
    fn explicit_quickshell_without_launcher_falls_back_with_notice() {
        // The configuration that hid the OSD mismatch: quickshell
        // requested, launcher not next to the wrapper nor on PATH.
        let s = select(
            Quickshell,
            OMARCHY_WITH_QS,
            installed(&["voxtype-osd-gtk4", "voxtype-osd-native"]),
        );
        assert_eq!(s.chosen_frontend(), Some(Gtk4));
        assert_eq!(s.preferred_unavailable, Some(Unavailable::NotInstalled));
        let n = s.notice().unwrap();
        assert_eq!(n.title, "Voxtype: OSD frontend 'quickshell' unavailable");
        assert!(
            n.body.contains("Showing the gtk4 OSD instead."),
            "{}",
            n.body
        );
        assert!(
            n.body.contains("voxtype-osd-quickshell was not found"),
            "{}",
            n.body
        );
        assert!(n.body.contains("[osd] frontend = \"gtk4\""), "{}", n.body);
    }

    #[test]
    fn explicit_quickshell_without_qs_falls_back_with_install_hint() {
        let s = select(Quickshell, PLAIN, installed(ALL));
        assert_eq!(s.chosen_frontend(), Some(Gtk4));
        assert_eq!(
            s.preferred_unavailable,
            Some(Unavailable::QuickshellMissing)
        );
        let n = s.notice().unwrap();
        assert!(n.body.contains("sudo pacman -S quickshell"), "{}", n.body);
    }

    #[test]
    fn explicit_gtk4_missing_names_its_feature() {
        let s = select(Gtk4, PLAIN, installed(&["voxtype-osd-native"]));
        assert_eq!(s.chosen_frontend(), Some(Native));
        let n = s.notice().unwrap();
        assert!(n.body.contains("--features osd-gtk4"), "{}", n.body);
    }

    #[test]
    fn nothing_installed_is_always_reported() {
        for requested in [Auto, Gtk4, Quickshell] {
            let s = select(requested, OMARCHY_WITH_QS, installed(&[]));
            assert_eq!(s.chosen, None);
            let n = s.notice().unwrap();
            assert_eq!(n.title, "Voxtype: no OSD frontend installed");
        }
    }

    #[test]
    fn chains_start_with_the_preferred_frontend() {
        for f in [Gtk4, Native, Quickshell] {
            assert_eq!(fallback_chain(f)[0], f);
        }
        assert_eq!(fallback_chain(Auto), fallback_chain(Gtk4));
    }

    impl Environment {
        fn with_qs(self) -> Self {
            Self {
                qs_installed: true,
                ..self
            }
        }
    }
}
