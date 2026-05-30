//! Engine-vs-running-binary mismatch detection.
//!
//! The wrapper at `/usr/bin/voxtype` can dispatch to any of the installed
//! variants (CPU Whisper, GPU Whisper, or one of the ONNX variants). When
//! the variant the wrapper points at doesn't ship the engine the user has
//! configured, the daemon fails with `TranscribeError::InitFailed` at
//! startup and `voxtype models` lists "rebuild with --features X" for every
//! engine the binary lacks.
//!
//! Two failure modes converge here:
//!
//! 1. A user upgraded from 0.6.x to 0.7.0 before the rename map landed in
//!    voxtype-bin's post-upgrade hook (#446 / Discord/Ryan). Their saved
//!    backend path `voxtype-parakeet-avx2` no longer existed, the hook
//!    fell through to `_set_default_backend`, and they've been silently
//!    running CPU Whisper despite `engine = "parakeet"` in their config.
//!
//! 2. A new user followed the AUR install instructions, kept the default
//!    CPU variant, then edited `engine = "parakeet"` in their config. They
//!    get the same end state without realizing they also need
//!    `voxtype setup onnx --enable`.
//!
//! Both cases were invisible — `voxtype configure` opened cleanly to
//! whichever section the user was after, and the engine-section warning
//! sat behind a navigation away from the section the user opened. This
//! module exposes the detection as a pure function so it can drive a
//! global TUI banner, a daemon startup notification, and the Active
//! Variant TUI switcher screen — all reading the same truth.

use crate::config::{Config, TranscriptionEngine};
use crate::setup::binary::{Inventory, Variant};

/// What's wrong when the running binary can't serve the configured engine.
///
/// `None` means everything checks out: either the engine is Whisper (always
/// available) or the running binary's `compiled_features` includes the
/// engine's feature flag.
#[derive(Debug, Clone)]
pub struct VariantMismatch {
    /// The engine the user has selected, e.g. `"parakeet"`. Lowercase
    /// kebab-case so it can drop straight into the banner sentence.
    pub configured_engine: &'static str,
    /// The Cargo feature flag that would have to be compiled into a binary
    /// for this engine to work, e.g. `"parakeet"`. Same string as
    /// `configured_engine` for every engine today, but kept separate so
    /// future engines whose feature name diverges from the engine name
    /// (or that gate on a meta-feature) can be expressed cleanly.
    pub required_feature: &'static str,
    /// Basename of the binary the wrapper currently dispatches to, e.g.
    /// `"voxtype-avx2"`. `None` only on source builds (no wrapper).
    pub active_variant_name: Option<String>,
    /// Recommended fix; varies by install kind. See [`Remediation`].
    pub remediation: Remediation,
}

/// How to fix the mismatch, tailored to how voxtype was installed.
///
/// `Source` and `Package` are very different recovery paths: package users
/// switch variants with one command, source users have to rebuild the
/// binary. A TUI banner should phrase both correctly.
#[derive(Debug, Clone)]
pub enum Remediation {
    /// Package install (voxtype-bin / .deb / .rpm / Nix). Swap the
    /// `/usr/bin/voxtype` dispatch to the listed variant. `target` is the
    /// recommended ONNX variant for this host's hardware
    /// (`Inventory::recommendation.onnx`); the variant switcher screen
    /// uses it as the default selection.
    SwitchToVariant { target: Variant },
    /// Source install (cargo build from a git checkout). The user has to
    /// rebuild — there's no separate ONNX variant to swap to. `feature`
    /// is the Cargo feature to add to the build, e.g. `"parakeet"`.
    Rebuild { feature: &'static str },
}

/// Map a `TranscriptionEngine` to its required Cargo feature, or `None`
/// for engines that have no compile-time gate.
///
/// Whisper is unconditional in every variant (the engine itself is the
/// reason whisper-rs is a non-optional dependency). Every other engine is
/// behind a feature flag of the same name — so once Whisper is excluded
/// the feature name is just the engine's canonical name.
pub fn required_feature(engine: TranscriptionEngine) -> Option<&'static str> {
    match engine {
        TranscriptionEngine::Whisper => None,
        other => Some(other.name()),
    }
}

/// Returns `Some` when the configured engine isn't available in the running
/// binary's compiled features, `None` when everything matches up.
///
/// The check looks only at the *running* binary's `compiled_features` — not
/// at what's installed under `/usr/lib/voxtype/`. A package install with the
/// right ONNX variant sitting on disk but the wrapper pointing at the CPU
/// Whisper variant still trips the mismatch, because the running CLI / TUI
/// invocation is the CPU binary and the daemon `systemctl --user restart
/// voxtype` invokes the same wrapper. Surfacing this is the whole point of
/// the check — the data is already on disk; the user just doesn't know.
pub fn detect_mismatch(config: &Config, inventory: &Inventory) -> Option<VariantMismatch> {
    let engine = config.engine;
    let feature = required_feature(engine)?;
    if inventory.compiled_features.contains(&feature) {
        return None;
    }

    let active_variant_name = inventory
        .active_variant
        .map(|v| v.binary_name().to_string());

    let remediation = match inventory.install_kind {
        crate::setup::binary::InstallKind::Source => Remediation::Rebuild { feature },
        // Package installs: recommend the hardware-appropriate ONNX variant.
        // The Inventory's recommendation pass already picked the best one
        // (CUDA-12/13 vs MIGraphX vs AVX-512 vs AVX2) given the detected
        // CPU/GPU.
        _ => Remediation::SwitchToVariant {
            target: inventory.recommendation.onnx,
        },
    };

    Some(VariantMismatch {
        configured_engine: engine.name(),
        required_feature: feature,
        active_variant_name,
        remediation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::binary::{Cpu, EngineFamily, Gpus, InstallKind, Recommendation, Variant};

    fn fake_inventory(
        install_kind: InstallKind,
        compiled_features: Vec<&'static str>,
        active_variant: Variant,
        recommendation_onnx: Variant,
    ) -> Inventory {
        Inventory {
            install_kind,
            binary_path: "/usr/bin/voxtype".into(),
            package_lib_dir: Some("/usr/lib/voxtype".into()),
            active_variant: Some(active_variant),
            variants: vec![],
            cpu: Cpu {
                avx2: true,
                avx512: false,
            },
            gpus: Gpus {
                nvidia: false,
                amd: false,
            },
            compiled_features,
            recommendation: Recommendation {
                whisper: Variant::WhisperAvx2,
                whisper_reason: "test",
                onnx: recommendation_onnx,
                onnx_reason: "test",
                primary: Variant::WhisperAvx2,
            },
        }
    }

    fn config_with_engine(engine: TranscriptionEngine) -> Config {
        Config {
            engine,
            ..Config::default()
        }
    }

    #[test]
    fn whisper_engine_never_mismatches() {
        let inv = fake_inventory(
            InstallKind::Package,
            vec![], // no ONNX features at all
            Variant::WhisperAvx2,
            Variant::OnnxAvx2,
        );
        let cfg = config_with_engine(TranscriptionEngine::Whisper);
        assert!(detect_mismatch(&cfg, &inv).is_none());
    }

    #[test]
    fn parakeet_on_cpu_whisper_binary_is_mismatch() {
        // The Ryan case from Discord: engine = parakeet, running binary
        // is the CPU Whisper variant with no ONNX features compiled in.
        let inv = fake_inventory(
            InstallKind::Package,
            vec![], // only whisper, no parakeet
            Variant::WhisperAvx2,
            Variant::OnnxAvx2,
        );
        let cfg = config_with_engine(TranscriptionEngine::Parakeet);
        let m = detect_mismatch(&cfg, &inv).expect("should detect mismatch");
        assert_eq!(m.configured_engine, "parakeet");
        assert_eq!(m.required_feature, "parakeet");
        assert_eq!(m.active_variant_name.as_deref(), Some("voxtype-avx2"));
        match m.remediation {
            Remediation::SwitchToVariant { target } => {
                assert_eq!(target, Variant::OnnxAvx2);
                assert_eq!(target.family(), EngineFamily::Onnx);
            }
            Remediation::Rebuild { .. } => panic!("package install should recommend variant swap"),
        }
    }

    #[test]
    fn parakeet_on_onnx_binary_is_not_mismatch() {
        let inv = fake_inventory(
            InstallKind::Package,
            vec!["parakeet", "moonshine", "sensevoice", "cohere"],
            Variant::OnnxAvx2,
            Variant::OnnxAvx2,
        );
        let cfg = config_with_engine(TranscriptionEngine::Parakeet);
        assert!(detect_mismatch(&cfg, &inv).is_none());
    }

    #[test]
    fn source_install_recommends_rebuild() {
        let inv = fake_inventory(
            InstallKind::Source,
            vec![], // source build without --features cohere
            Variant::WhisperNative,
            Variant::OnnxNative,
        );
        let cfg = config_with_engine(TranscriptionEngine::Cohere);
        let m = detect_mismatch(&cfg, &inv).expect("should detect");
        match m.remediation {
            Remediation::Rebuild { feature } => assert_eq!(feature, "cohere"),
            Remediation::SwitchToVariant { .. } => {
                panic!("source install should recommend rebuild, not variant swap")
            }
        }
    }

    #[test]
    fn every_non_whisper_engine_has_a_required_feature() {
        // Regression guard: if a new engine is added to TranscriptionEngine
        // without updating required_feature, this catches it before it
        // ships as a silent always-passes mismatch check.
        let engines = [
            TranscriptionEngine::Parakeet,
            TranscriptionEngine::Moonshine,
            TranscriptionEngine::SenseVoice,
            TranscriptionEngine::Paraformer,
            TranscriptionEngine::Dolphin,
            TranscriptionEngine::Omnilingual,
            TranscriptionEngine::Cohere,
            TranscriptionEngine::Soniox,
        ];
        for e in engines {
            assert!(
                required_feature(e).is_some(),
                "engine {:?} must declare a required Cargo feature",
                e
            );
        }
        assert_eq!(required_feature(TranscriptionEngine::Whisper), None);
    }
}
