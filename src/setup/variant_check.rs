//! Engine-vs-running-binary mismatch detection.
//!
//! `/usr/bin/voxtype` dispatches to one installed variant (CPU Whisper,
//! GPU Whisper, or an ONNX variant). When that variant lacks the engine
//! the user configured, the daemon fails to initialize the transcriber
//! and `voxtype models` lists "rebuild with --features X" for every
//! engine the binary doesn't have.
//!
//! Exposed as a pure function so the TUI banner, the daemon startup
//! notification, and any future surface read the same truth.

use crate::config::{Config, TranscriptionEngine};
use crate::setup::binary::{Inventory, Variant};

/// What's wrong when the running binary can't serve the configured engine.
///
/// `None` means everything checks out: either the engine is always available
/// (Whisper or Soniox) or the running binary's `compiled_features` includes the
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

impl VariantMismatch {
    /// One-paragraph explanation plus fix, shared by the daemon's startup
    /// notification and `voxtype doctor`. The fix comes from
    /// [`crate::setup::install_hint`], so it names the command for how
    /// voxtype was installed on this machine.
    pub fn explanation(&self) -> String {
        use crate::setup::install_hint::{self, Need};
        self.explanation_with(&install_hint::hint(Need::Feature(self.required_feature)))
    }

    /// [`Self::explanation`] with the install's remediation already worked
    /// out, so each wording can be tested without this machine's install.
    pub fn explanation_with(&self, hint: &crate::setup::install_hint::Hint) -> String {
        use crate::setup::install_hint::Hint;

        let active = self
            .active_variant_name
            .as_deref()
            .unwrap_or("the running binary");
        match (&self.remediation, hint) {
            (_, Hint::MacosLinuxOnly) => format!(
                "voxtype for macOS does not include the {} engine. To use it, {}.",
                self.configured_engine, hint,
            ),
            (Remediation::SwitchToVariant { target: None }, _) => format!(
                "{} was built without --features {}, and the builds that have it need a CPU \
                 with AVX2, which this one lacks. Choose an engine the running build supports.",
                active, self.required_feature,
            ),
            (
                Remediation::SwitchToVariant {
                    target: Some(target),
                },
                Hint::Switch { command },
            ) => format!(
                "{} was built without --features {}. \
                 Run `{}` (or open `voxtype configure` and press F2) \
                 to switch to {}.",
                active,
                self.required_feature,
                command,
                target.binary_name(),
            ),
            (Remediation::SwitchToVariant { .. }, hint) => format!(
                "{} was built without --features {}, and no installed variant has it. \
                 To enable the {} engine, {}.",
                active, self.required_feature, self.configured_engine, hint,
            ),
            (Remediation::Rebuild { feature }, hint) => format!(
                "This source build was compiled without --features {}. \
                 To enable the {} engine, {}.",
                feature, self.configured_engine, hint,
            ),
            (Remediation::NixPackage { feature }, hint) => format!(
                "This voxtype flake package was built without --features {}. \
                 To enable the {} engine, {}.",
                feature, self.configured_engine, hint,
            ),
        }
    }
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
    /// uses it as the default selection. `None` when no ONNX build runs on
    /// this CPU (no AVX2), so there is nothing to switch to.
    SwitchToVariant { target: Option<Variant> },
    /// Source install (cargo build from a git checkout). The user has to
    /// rebuild — there's no separate ONNX variant to swap to. `feature`
    /// is the Cargo feature to add to the build, e.g. `"parakeet"`.
    Rebuild { feature: &'static str },
    /// Nix install (a voxtype flake package in the read-only store). There
    /// is nothing to rebuild or switch in place; the fix is another flake
    /// package. `feature` is the Cargo feature the engine needs.
    NixPackage { feature: &'static str },
}

/// Map a `TranscriptionEngine` to its required Cargo feature, or `None`
/// for engines that have no compile-time gate.
///
/// Whisper and Soniox are unconditional in every variant. The remaining
/// engines are behind feature flags matching their canonical names.
pub fn required_feature(engine: TranscriptionEngine) -> Option<&'static str> {
    match engine {
        TranscriptionEngine::Whisper | TranscriptionEngine::Soniox => None,
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
        crate::setup::binary::InstallKind::Source
            if crate::setup::install_hint::is_nix_store_path(&inventory.binary_path) =>
        {
            Remediation::NixPackage { feature }
        }
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
            // This fixture drives the variant-mismatch banner, which reads the
            // symlink's variant; the running-daemon fields are not part of it.
            running_variant: None,
            daemon_pid: None,
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
                onnx: Some(recommendation_onnx),
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
    fn soniox_engine_never_mismatches() {
        // #873: Soniox has no Cargo feature, so no variant may warn or
        // recommend switching binaries for it.
        for variant in [
            Variant::WhisperAvx2,
            Variant::WhisperVulkan,
            Variant::OnnxAvx2,
        ] {
            let inv = fake_inventory(InstallKind::Package, vec![], variant, Variant::OnnxAvx2);
            let cfg = config_with_engine(TranscriptionEngine::Soniox);
            assert!(detect_mismatch(&cfg, &inv).is_none());
        }
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
                assert_eq!(target, Some(Variant::OnnxAvx2));
                assert_eq!(target.map(Variant::family), Some(EngineFamily::Onnx));
            }
            other => panic!("package install should recommend variant swap, got {other:?}"),
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
            other => panic!("source install should recommend rebuild, got {other:?}"),
        }
    }

    #[test]
    fn nix_install_recommends_a_flake_package_not_a_rebuild() {
        let mut inv = fake_inventory(
            InstallKind::Source,
            vec!["gpu-vulkan"],
            Variant::WhisperNative,
            Variant::OnnxAvx2,
        );
        inv.binary_path = "/nix/store/abc-voxtype-vulkan-1.3.1/bin/voxtype".into();
        inv.package_lib_dir = None;
        let cfg = config_with_engine(TranscriptionEngine::Parakeet);
        let m = detect_mismatch(&cfg, &inv).expect("should detect");
        assert!(matches!(
            m.remediation,
            Remediation::NixPackage {
                feature: "parakeet"
            }
        ));
        let text = m.explanation_with(&crate::setup::install_hint::Hint::Nix {
            package: Some("onnx"),
            feature: Some("parakeet"),
        });
        assert!(
            text.starts_with("This voxtype flake package was built without --features parakeet."),
            "{text}"
        );
        assert!(text.contains("voxtype.packages.${system}.onnx"), "{text}");
        assert!(!text.contains("source build"), "{text}");
        assert!(!text.contains("rebuild"), "{text}");
    }

    #[test]
    fn explanation_names_the_package_fix_for_package_installs() {
        use crate::setup::install_hint::Hint;
        let inv = fake_inventory(
            InstallKind::Package,
            vec![],
            Variant::WhisperAvx2,
            Variant::OnnxAvx2,
        );
        let m = detect_mismatch(&config_with_engine(TranscriptionEngine::Moonshine), &inv)
            .expect("should detect");

        let switch = m.explanation_with(&Hint::Switch {
            command: "sudo voxtype setup onnx --enable",
        });
        assert_eq!(
            switch,
            "voxtype-avx2 was built without --features moonshine. \
             Run `sudo voxtype setup onnx --enable` (or open `voxtype configure` and press F2) \
             to switch to voxtype-onnx-avx2."
        );

        let aur = m.explanation_with(&Hint::AurVariant {
            token: "onnx-avx2".into(),
        });
        assert!(aur.contains("no installed variant has it"), "{aur}");
        assert!(
            aur.contains("VOXTYPE_VARIANTS=onnx-avx2 yay -S --rebuild voxtype-bin"),
            "{aur}"
        );
        assert!(!aur.contains("--features moonshine."), "{aur}");

        let reinstall = m.explanation_with(&Hint::Reinstall {
            method: crate::model_upgrade::InstallMethod::Dpkg("voxtype".into()),
        });
        assert!(
            reinstall.contains("sudo apt install --reinstall voxtype"),
            "{reinstall}"
        );
        assert!(!reinstall.contains("source build"), "{reinstall}");
    }

    #[test]
    fn explanation_says_rebuild_only_for_source_builds() {
        let inv = fake_inventory(
            InstallKind::Source,
            vec![],
            Variant::WhisperNative,
            Variant::OnnxNative,
        );
        let m = detect_mismatch(&config_with_engine(TranscriptionEngine::Cohere), &inv)
            .expect("should detect");
        assert_eq!(
            m.explanation_with(&crate::setup::install_hint::Hint::Rebuild {
                feature: Some("cohere")
            }),
            "This source build was compiled without --features cohere. \
             To enable the cohere engine, rebuild with --features cohere."
        );
    }

    /// #632: on macOS the explanation names the macOS engines whatever the
    /// remediation, Nix and source included, and never suggests a rebuild.
    #[test]
    fn macos_hint_wins_over_every_remediation() {
        let cfg = config_with_engine(TranscriptionEngine::Parakeet);
        let mut source = fake_inventory(
            InstallKind::Source,
            vec![],
            Variant::WhisperNative,
            Variant::OnnxNative,
        );
        let package = fake_inventory(
            InstallKind::Package,
            vec![],
            Variant::WhisperAvx2,
            Variant::OnnxAvx2,
        );
        let from_source = detect_mismatch(&cfg, &source).expect("source");
        source.binary_path = "/nix/store/abc-voxtype-1.3.1/bin/voxtype".into();
        source.package_lib_dir = None;
        let from_nix = detect_mismatch(&cfg, &source).expect("nix");
        assert!(matches!(
            from_nix.remediation,
            Remediation::NixPackage { .. }
        ));
        let from_package = detect_mismatch(&cfg, &package).expect("package");
        let no_avx2 = VariantMismatch {
            remediation: Remediation::SwitchToVariant { target: None },
            ..from_package.clone()
        };

        for m in [from_source, from_nix, from_package, no_avx2] {
            let text = m.explanation_with(&crate::setup::install_hint::Hint::MacosLinuxOnly);
            assert!(
                text.starts_with("voxtype for macOS does not include the parakeet engine."),
                "{text}"
            );
            assert!(text.contains("Linux-only for now"), "{text}");
            assert!(!text.contains("--features"), "{text}");
            assert!(!text.contains("flake"), "{text}");
        }
    }

    #[test]
    fn every_optional_engine_has_a_required_feature() {
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
        ];
        for e in engines {
            assert!(
                required_feature(e).is_some(),
                "engine {:?} must declare a required Cargo feature",
                e
            );
        }
        assert_eq!(required_feature(TranscriptionEngine::Whisper), None);
        assert_eq!(required_feature(TranscriptionEngine::Soniox), None);
    }
}
