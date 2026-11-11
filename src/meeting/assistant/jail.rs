//! A filesystem jail for codex.
//!
//! codex keeps a command tool that cannot be switched off (see
//! `agent::Support::Jailed`), and the transcript in its prompt is
//! untrusted: remote participants' words go straight in. A read-only tool
//! that can read anywhere would let a spoken prompt injection pull
//! `~/.ssh` or `~/.config` into the model's context. So codex only ever runs
//! under bubblewrap, where it sees:
//!
//! - read-only: `/usr` and the host's `/bin`, `/lib*` layout; the few `/etc`
//!   entries name resolution and TLS need; the directory holding the codex
//!   binary; and `auth.json` from its CODEX_HOME;
//! - an empty HOME (a fresh CODEX_HOME inside it for codex's own logs and
//!   state) and a tmpfs `/tmp` with an empty working directory, all of it
//!   discarded when codex exits;
//! - one writable directory, where it writes the answer;
//! - the network, which it needs for the API.
//!
//! Nothing else from the real HOME exists in the jail. Without `bwrap`,
//! codex is not used.

use std::path::{Path, PathBuf};

/// Where things are inside the jail.
/// Not under /tmp: codex refuses to set up its helper binaries in a
/// temporary directory and warns on every run.
pub const JAIL_HOME: &str = "/home/voxtype-jail";
pub const JAIL_CODEX_HOME: &str = "/home/voxtype-jail/.codex";
pub const JAIL_CWD: &str = "/tmp/work";
pub const JAIL_OUT: &str = "/tmp/out";

/// Environment passed into the jail when set: proxies, certificate
/// overrides and codex's own API key variables. Everything else is cleared.
const PASS_ENV: &[&str] = &[
    "LANG",
    "LC_ALL",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
    "ALL_PROXY",
    "all_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
];

/// `/etc` entries for DNS, users and TLS roots. Missing ones are skipped.
const ETC: &[&str] = &[
    "/etc/resolv.conf",
    "/etc/hosts",
    "/etc/nsswitch.conf",
    "/etc/host.conf",
    "/etc/gai.conf",
    "/etc/passwd",
    "/etc/group",
    "/etc/localtime",
    "/etc/ssl",
    "/etc/ca-certificates",
    "/etc/pki",
];

/// Everything needed to run codex jailed.
#[derive(Debug, Clone, PartialEq)]
pub struct CodexJail {
    pub bwrap: PathBuf,
    /// The program run inside: the native codex binary, or a script whose
    /// interpreter is a system one.
    pub program: PathBuf,
    /// Bound read-only at its own path.
    pub install_dir: PathBuf,
    /// The user's `auth.json`, bound read-only into the jail's CODEX_HOME.
    pub auth: Option<PathBuf>,
}

fn is_elf(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && magic == *b"\x7fELF"
}

/// The real codex behind what PATH found: through symlinks, through a mise
/// shim (`mise which codex`), and from an npm wrapper script to the native
/// binary it ships under `vendor/`.
pub fn real_codex(found: &Path, run: &dyn Fn(&Path, &[&str]) -> Option<String>) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(found).ok()?;
    if canonical.file_name().is_some_and(|n| n == "mise") {
        let real = run(&canonical, &["which", "codex"])?;
        return real_codex(Path::new(real.trim()), run)
            .filter(|p| p.file_name().is_some_and(|n| n != "mise"));
    }
    if is_elf(&canonical) {
        return Some(canonical);
    }
    // npm installs a node launcher; the native binary sits beside it in the
    // package. Prefer it, so node and the package tree stay outside.
    if let Some(pkg) = canonical.parent().and_then(Path::parent) {
        if let Ok(entries) = std::fs::read_dir(pkg.join("vendor")) {
            for e in entries.flatten() {
                let native = e.path().join("codex").join("codex");
                if is_elf(&native) {
                    return Some(native);
                }
            }
        }
    }
    // A script with a system interpreter runs as-is.
    let head = std::fs::read(&canonical).ok()?;
    let first = head.split(|b| *b == b'\n').next().unwrap_or_default();
    let first = String::from_utf8_lossy(first);
    let interp = first.strip_prefix("#!")?.split_whitespace().next()?;
    (interp.starts_with("/usr/") || interp.starts_with("/bin/")).then_some(canonical)
}

/// Set up the jail for the codex found at `found`, or say why it can't be.
pub fn locate(
    bwrap: Option<PathBuf>,
    found: &Path,
    codex_home: &Path,
    run: &dyn Fn(&Path, &[&str]) -> Option<String>,
) -> Result<CodexJail, String> {
    let bwrap = bwrap.ok_or_else(|| {
        "needs bubblewrap (bwrap) to run codex in a filesystem jail; install bubblewrap".to_string()
    })?;
    let program = real_codex(found, run).ok_or_else(|| {
        format!(
            "could not find the codex program behind {} to run in a jail",
            found.display()
        )
    })?;
    let install_dir = program
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "codex has no install directory".to_string())?;
    let auth = Some(codex_home.join("auth.json")).filter(|p| p.is_file());
    Ok(CodexJail {
        bwrap,
        program,
        install_dir,
        auth,
    })
}

impl CodexJail {
    /// bwrap's arguments: the jail, then `codex_args` for the program.
    /// `out_dir` is bound writable at [`JAIL_OUT`].
    pub fn args(&self, codex_args: &[String], out_dir: &Path) -> Vec<String> {
        let mut a: Vec<String> = Vec::new();
        let mut push = |items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));
        push(&[
            "--unshare-all",
            "--share-net",
            "--die-with-parent",
            "--new-session",
            "--clearenv",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--ro-bind",
            "/usr",
            "/usr",
        ]);
        // Mirror the host's top-level layout: symlinks into /usr on merged-usr
        // systems, read-only directories elsewhere.
        for top in ["/bin", "/sbin", "/lib", "/lib64", "/lib32"] {
            let p = Path::new(top);
            match std::fs::read_link(p) {
                Ok(target) => {
                    let t = target.display().to_string();
                    push(&["--symlink", &t, top]);
                }
                Err(_) if p.is_dir() => push(&["--ro-bind", top, top]),
                Err(_) => {}
            }
        }
        for etc in ETC {
            if Path::new(etc).exists() {
                push(&["--ro-bind", etc, etc]);
            }
        }
        push(&[
            "--tmpfs",
            "/tmp",
            "--dir",
            JAIL_HOME,
            "--dir",
            JAIL_CODEX_HOME,
            "--dir",
            JAIL_CWD,
        ]);
        let install = self.install_dir.display().to_string();
        push(&["--ro-bind", &install, &install]);
        if let Some(auth) = &self.auth {
            let auth = auth.display().to_string();
            push(&["--ro-bind", &auth, &format!("{JAIL_CODEX_HOME}/auth.json")]);
        }
        let out = out_dir.display().to_string();
        push(&["--bind", &out, JAIL_OUT]);
        push(&[
            "--setenv",
            "HOME",
            JAIL_HOME,
            "--setenv",
            "CODEX_HOME",
            JAIL_CODEX_HOME,
            "--setenv",
            "PATH",
            "/usr/local/bin:/usr/bin:/bin",
            "--setenv",
            "NO_COLOR",
            "1",
            "--setenv",
            "TERM",
            "dumb",
        ]);
        for name in PASS_ENV {
            if let Ok(v) = std::env::var(name) {
                push(&["--setenv", name, &v]);
            }
        }
        push(&["--chdir", JAIL_CWD, "--"]);
        a.push(self.program.display().to_string());
        a.extend(codex_args.iter().cloned());
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_run(_: &Path, _: &[&str]) -> Option<String> {
        None
    }

    #[test]
    fn without_bwrap_codex_cannot_be_jailed() {
        let err = locate(None, Path::new("/usr/bin/sh"), Path::new("/x"), &no_run).unwrap_err();
        assert!(err.contains("bubblewrap"), "{err}");
    }

    #[test]
    fn a_mise_shim_resolves_to_the_real_binary() {
        let dir = tempfile::tempdir().unwrap();
        let mise = dir.path().join("mise");
        std::fs::copy("/bin/sh", &mise).unwrap();
        let shim = dir.path().join("codex");
        std::os::unix::fs::symlink(&mise, &shim).unwrap();
        let real = std::fs::canonicalize("/bin/sh").unwrap();
        let real_s = real.display().to_string();
        let run = move |exe: &Path, args: &[&str]| {
            assert!(exe.ends_with("mise"));
            assert_eq!(args, ["which", "codex"]);
            Some(format!("{real_s}\n"))
        };
        assert_eq!(real_codex(&shim, &run), Some(real));
    }

    #[test]
    fn the_jail_binds_only_what_codex_needs() {
        let jail = CodexJail {
            bwrap: "/usr/bin/bwrap".into(),
            program: "/opt/codex/bin/codex".into(),
            install_dir: "/opt/codex/bin".into(),
            auth: Some("/home/u/.codex/auth.json".into()),
        };
        let a = jail.args(&["exec".into(), "-".into()], Path::new("/tmp/x/out"));
        let joined = a.join(" ");
        assert!(joined.contains("--ro-bind /opt/codex/bin /opt/codex/bin"));
        assert!(joined
            .contains("--ro-bind /home/u/.codex/auth.json /home/voxtype-jail/.codex/auth.json"));
        assert!(joined.contains("--bind /tmp/x/out /tmp/out"));
        assert!(joined.contains("--clearenv"));
        assert!(joined.contains("--share-net"));
        assert!(joined.ends_with("-- /opt/codex/bin/codex exec -"));
        // Nothing from the real home but auth.json.
        let home_binds: Vec<&String> = a.iter().filter(|s| s.starts_with("/home/u/")).collect();
        assert_eq!(home_binds, vec!["/home/u/.codex/auth.json"]);
        assert!(!a.iter().any(|s| s == "/"));
    }
}
