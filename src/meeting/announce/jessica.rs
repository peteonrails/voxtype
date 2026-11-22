//! The "jessica" announcement voice: an ElevenLabs rendering of
//! [`super::ANNOUNCEMENT_TEXT`], downloaded once on request.
//!
//! It is not in the repository or the binary, because its terms are not
//! MIT: it may be redistributed only unmodified, as part of voxtype, and may
//! not be used to train or improve AI models. Voxtype fetches it from the
//! models CDN the first time it is wanted (`voxtype meeting announcement
//! --voice jessica`, or a meeting that announces with that voice), checks it
//! against the sha256 pinned here, and keeps it in
//! `$XDG_DATA_HOME/voxtype/sounds/`. A fetch that fails leaves the built-in
//! voice to play; a meeting never fails over it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The clip's path on the models CDN.
pub const OBJECT: &str = "sounds/meeting-announcement-jessica-v1.wav";
/// The clip's sha256, as rendered. Only a file with exactly this content is
/// ever played.
pub const SHA256: &str = "40267063245522143aeca0793f5a435e583beade0194efb041bfb2513cea7251";
/// The clip's size in bytes.
pub const SIZE: u64 = 153_852;
/// The cached clip's file name under [`cache_dir`].
pub const CACHE_FILE: &str = "meeting-announcement-jessica.wav";
/// The terms, written beside the cached clip.
pub const TERMS_FILE: &str = "meeting-announcement-jessica.LICENSE.txt";
/// How long a meeting waits for the download before it plays the built-in
/// voice instead.
pub const MEETING_TIMEOUT: Duration = Duration::from_secs(15);
/// How long `voxtype meeting announcement --voice jessica` waits.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// What the user is told before the clip is first used.
pub const TERMS: &str = "The Jessica announcement voice is AI-generated speech, rendered with \
ElevenLabs text-to-speech (voice: Jessica). It is not covered by voxtype's MIT licence: it may \
be redistributed only unmodified, as part of voxtype or a package of voxtype, and it may not be \
used to train or improve AI models.";

/// The clip's URL on the models CDN.
pub fn url() -> String {
    format!("{}/{}", crate::setup::manifest::MODELS_BASE_URL, OBJECT)
}

/// Where downloaded announcement voices are kept.
pub fn cache_dir() -> PathBuf {
    crate::config::Config::data_dir().join("sounds")
}

/// The cached clip's path, whether or not it has been downloaded.
pub fn cache_path() -> PathBuf {
    cache_dir().join(CACHE_FILE)
}

/// Whether `path` holds the clip: present, and with the pinned sha256.
pub fn verify(path: &Path) -> Result<(), String> {
    verify_sha(path, SHA256)
}

/// [`verify`] against `sha256`.
pub(crate) fn verify_sha(path: &Path, sha256: &str) -> Result<(), String> {
    if !path.is_file() {
        return Err("not downloaded".into());
    }
    let hash = crate::setup::model::sha256_file(path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    if hash != sha256 {
        return Err(format!(
            "{} does not match the published clip (sha256 {})",
            path.display(),
            hash
        ));
    }
    Ok(())
}

/// Download the clip from `url` to `dest`, giving up after `timeout`. It
/// reaches `dest` only once its sha256 matches; anything else is deleted
/// and reported. The terms are written beside it.
pub fn fetch(url: &str, dest: &Path, timeout: Duration) -> Result<(), String> {
    fetch_sha(url, dest, timeout, SHA256)
}

/// [`fetch`], checking against `sha256`.
pub(crate) fn fetch_sha(
    url: &str,
    dest: &Path,
    timeout: Duration,
    sha256: &str,
) -> Result<(), String> {
    let dir = dest
        .parent()
        .ok_or_else(|| format!("{} has no folder", dest.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;
    let part = dest.with_extension("wav.part");
    let _ = std::fs::remove_file(&part);
    let out = Command::new("curl")
        .arg("-fsSL")
        .arg("--max-time")
        .arg(timeout.as_secs().max(1).to_string())
        .arg("--max-filesize")
        .arg((SIZE * 4).to_string())
        .arg("-o")
        .arg(&part)
        .arg(url)
        .output()
        .map_err(|e| format!("cannot run curl: {}", e))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&part);
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if out.status.code() == Some(28) {
            format!("timed out after {} s", timeout.as_secs())
        } else if why.is_empty() {
            format!("curl exited with {}", out.status.code().unwrap_or(-1))
        } else {
            why
        });
    }
    if let Err(e) = verify_sha(&part, sha256) {
        let _ = std::fs::remove_file(&part);
        return Err(format!("rejected the download: {}", e));
    }
    std::fs::rename(&part, dest)
        .map_err(|e| format!("cannot move it to {}: {}", dest.display(), e))?;
    let _ = std::fs::write(dir.join(TERMS_FILE), format!("{TERMS}\n\nSource: {url}\n"));
    Ok(())
}

/// The clip ready to play at `dest`: the cached copy when it checks out,
/// else a fresh download through `download`, which is told where to put it.
/// Returns why it is not available otherwise.
pub fn ensure(
    dest: &Path,
    download: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    ensure_sha(dest, SHA256, download)
}

/// [`ensure`], checking against `sha256`.
pub(crate) fn ensure_sha(
    dest: &Path,
    sha256: &str,
    download: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    if verify_sha(dest, sha256).is_ok() {
        return Ok(());
    }
    // A damaged or tampered copy is never played; fetch it again.
    let _ = std::fs::remove_file(dest);
    download(dest)?;
    verify_sha(dest, sha256)
}
