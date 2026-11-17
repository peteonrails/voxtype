//! Explicit installation of the separately licensed LARD model.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MODEL_SHA256: &str = "f11b45d644b2d829349df2ac2ad3d69c9cf19cd3fae888c69268bddf64cfb354";
const MODEL_SIZE: u64 = 65_560;

pub fn model_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("voxtype/models/lard-disfluency-v1.vxdf")
}

fn validate(bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 != MODEL_SIZE || format!("{:x}", Sha256::digest(bytes)) != MODEL_SHA256 {
        bail!("LARD model checksum mismatch; use the pinned lard-disfluency-v1.vxdf artifact");
    }
    crate::text::disfluency::DisfluencyTagger::from_bytes(bytes)?;
    Ok(())
}

fn install(bytes: &[u8], destination: &Path) -> Result<()> {
    validate(bytes)?;
    let parent = destination
        .parent()
        .context("LARD model path has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    std::io::Write::write_all(&mut file, bytes)?;
    file.persist(destination)
        .context("Cannot install LARD model")?;
    Ok(())
}

/// No URL is enabled implicitly until model publication has been approved.
/// An explicit URL is still pinned to the exact release bytes, not arbitrary weights.
pub fn setup(source: Option<&Path>, url: Option<&str>, status: bool) -> Result<()> {
    let destination = model_path();
    if status {
        match std::fs::read(&destination).map_err(anyhow::Error::from).and_then(|bytes| validate(&bytes)) {
            Ok(()) => println!("LARD model installed and verified: {}", destination.display()),
            Err(error) => println!("LARD model unavailable: {error}. Install with voxtype setup lard --source <model.vxdf>"),
        }
        return Ok(());
    }
    let bytes = if let Some(source) = source {
        std::fs::read(source).context("Cannot read LARD model source")?
    } else if let Some(url) = url {
        if !url.starts_with("https://") {
            bail!("LARD model downloads require an explicit HTTPS URL");
        }
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(60))
            .build();
        let response = agent
            .get(url)
            .call()
            .context("LARD download failed; local cleanup remains available")?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MODEL_SIZE + 1)
            .read_to_end(&mut bytes)?;
        bytes
    } else {
        bail!("LARD model publication is pending approval. Use voxtype setup lard --source <model.vxdf> or --url <approved-https-url>; SHA-256: {MODEL_SHA256}");
    };
    install(&bytes, &destination)?;
    println!("LARD model installed: {}. Enable with voxtype config set text.lard_disfluency true (English only).", destination.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_install_rejects_corrupt_model_without_replacing_good_copy() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("model.vxdf");
        let model = include_bytes!("../../assets/models/lard-disfluency-v1.vxdf");
        install(model, &destination).unwrap();
        assert!(install(b"corrupt", &destination).is_err());
        assert_eq!(std::fs::read(destination).unwrap(), model);
    }
}
