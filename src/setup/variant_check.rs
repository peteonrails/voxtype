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

/// `gguf_backend` values that need a GPU backend compiled into the worker.
/// `auto`, `cpu` and `cpu_accel` run on any worker.
const GGUF_GPU_BACKENDS: [&str; 4] = ["vulkan", "cuda", "rocm", "metal"];

/// GPU backends the packaged GGUF worker named `worker_name` was built with,
/// or `None` for a worker this table does not know: a source build, a custom
/// `VOXTYPE_GGUF_WORKER`, the generic `voxtype-gguf-worker`, aarch64 or Nix.
///
/// Mirrors the release Dockerfiles. The Vulkan build and every x86_64 ONNX
/// build ship a worker built with `--features vulkan` (which
/// `scripts/verify-gguf-companion.sh --vulkan` checks links libvulkan), and
/// the CPU Whisper builds ship a CPU-only one. No packaged worker has CUDA,
/// ROCm or Metal.
pub fn packaged_worker_gpu_backends(worker_name: &str) -> Option<&'static [&'static str]> {
    match worker_name {
        "voxtype-gguf-worker-baseline"
        | "voxtype-gguf-worker-avx2"
        | "voxtype-gguf-worker-avx512" => Some(&[]),
        "voxtype-gguf-worker-vulkan"
        | "voxtype-gguf-worker-onnx-avx2"
        | "voxtype-gguf-worker-onnx-avx512"
        | "voxtype-gguf-worker-onnx-cuda-12"
        | "voxtype-gguf-worker-onnx-cuda-13"
        | "voxtype-gguf-worker-onnx-migraphx" => Some(&["vulkan"]),
        _ => None,
    }
}

/// The active GGUF model asks for a `gguf_backend` the worker of the running
/// build was not built with, so transcribe.cpp cannot bind it and the model
/// fails to load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GgufBackendMismatch {
    /// Config section of the GGUF engine, e.g. `"cohere"`.
    pub section: &'static str,
    /// The requested backend, lowercased, e.g. `"vulkan"`.
    pub requested: String,
    /// File name of the worker that would run the model, e.g.
    /// `"voxtype-gguf-worker-avx512"`.
    pub worker: String,
    /// GPU backends that worker was built with.
    pub worker_gpu_backends: &'static [&'static str],
}

impl GgufBackendMismatch {
    /// The requested backend as it reads in a sentence, e.g. "Vulkan".
    pub fn backend_label(&self) -> &str {
        match self.requested.as_str() {
            "vulkan" => "Vulkan",
            "cuda" => "CUDA",
            "rocm" => "ROCm",
            "metal" => "Metal",
            other => other,
        }
    }

    /// The build that ships this worker, e.g. "voxtype-avx512".
    fn build(&self) -> String {
        match self.worker.strip_prefix("voxtype-gguf-worker-") {
            Some(variant) => format!("voxtype-{variant}"),
            // The packaged generic worker belongs to the baseline build.
            None if self.worker == "voxtype-gguf-worker" => "voxtype-baseline".to_string(),
            None => "this voxtype build".to_string(),
        }
    }

    /// What is wrong, in one sentence.
    pub fn summary(&self) -> String {
        let built = if self.worker_gpu_backends.is_empty() {
            "runs on the CPU only".to_string()
        } else {
            format!("was built without {}", self.backend_label())
        };
        format!(
            "{section}.gguf_backend is \"{requested}\", but the GGUF worker of {build} ({worker}) \
             {built}, so the {section} model cannot load.",
            section = self.section,
            requested = self.requested,
            build = self.build(),
            worker = self.worker,
        )
    }

    /// How to fix it. `vulkan_build` is how to get onto the Vulkan build,
    /// from [`crate::setup::install_hint`].
    pub fn fix_with(&self, vulkan_build: &crate::setup::install_hint::Hint) -> String {
        let key = format!("{}.gguf_backend", self.section);
        let stay = format!("`voxtype config set {key} auto`");
        let worker_has_vulkan = self.worker_gpu_backends.contains(&"vulkan");
        match (self.requested.as_str(), worker_has_vulkan) {
            ("vulkan", _) => format!(
                "To run it on the GPU, {vulkan_build} and restart voxtype. \
                 To stay on this build, run {stay}."
            ),
            (_, true) => format!(
                "Voxtype's GGUF workers use Vulkan for the GPU, not {}. \
                 Run `voxtype config set {key} vulkan`, or {stay}.",
                self.backend_label()
            ),
            (_, false) => format!(
                "Voxtype's GGUF workers use Vulkan for the GPU, not {}. \
                 Run {stay} to use the CPU, or to run it on the GPU, {vulkan_build}, \
                 restart voxtype and run `voxtype config set {key} vulkan`.",
                self.backend_label()
            ),
        }
    }

    /// [`Self::fix_with`] for how voxtype is installed on this machine.
    pub fn fix(&self) -> String {
        use crate::setup::install_hint::{self, Need};
        self.fix_with(&install_hint::hint(Need::Variant(Variant::WhisperVulkan)))
    }

    /// Summary and fix, for the daemon's startup notice and
    /// `voxtype config validate-selection`.
    pub fn explanation(&self) -> String {
        format!("{} {}", self.summary(), self.fix())
    }
}

/// Returns `Some` when the active GGUF model requests a GPU backend that
/// `worker`, the worker that would run it, is known not to have. Workers the
/// table in [`packaged_worker_gpu_backends`] does not know are trusted.
pub fn detect_gguf_backend_mismatch(
    config: &Config,
    worker: &std::path::Path,
) -> Option<GgufBackendMismatch> {
    let active = config.active_gguf()?;
    let requested = active.backend.to_ascii_lowercase();
    if !GGUF_GPU_BACKENDS.contains(&requested.as_str()) {
        return None;
    }
    let in_package_dir = worker.parent() == Some(std::path::Path::new("/usr/lib/voxtype"));
    let worker = worker.file_name()?.to_str()?;
    // Packages install the baseline build's worker under the generic name,
    // and it is CPU-only. Elsewhere the generic name is a source build's.
    let worker_gpu_backends = if in_package_dir && worker == "voxtype-gguf-worker" {
        &[][..]
    } else {
        packaged_worker_gpu_backends(worker)?
    };
    if worker_gpu_backends.contains(&requested.as_str()) {
        return None;
    }
    Some(GgufBackendMismatch {
        section: active.section,
        requested,
        worker: worker.to_string(),
        worker_gpu_backends,
    })
}

/// [`detect_gguf_backend_mismatch`] for the worker this process would spawn.
/// A build without the GGUF client has no worker to ask about.
pub fn gguf_backend_mismatch(config: &Config) -> Option<GgufBackendMismatch> {
    #[cfg(any(feature = "cohere-gguf", feature = "parakeet-gguf", feature = "gguf"))]
    {
        detect_gguf_backend_mismatch(config, &crate::transcribe::gguf_subprocess::worker_path())
    }
    #[cfg(not(any(feature = "cohere-gguf", feature = "parakeet-gguf", feature = "gguf")))]
    {
        let _ = config;
        None
    }
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

    fn cohere_gguf(backend: &str) -> Config {
        let mut config = Config::default();
        config.set_model_for(
            TranscriptionEngine::Cohere,
            "cohere-transcribe-03-2026-Q4_K_M.gguf",
        );
        config.cohere.as_mut().unwrap().gguf_backend = backend.to_string();
        config
    }

    const SWITCH: crate::setup::install_hint::Hint = crate::setup::install_hint::Hint::Switch {
        command: "sudo voxtype setup gpu --enable",
    };

    /// voxtype-bin-rc left /usr/bin/voxtype on voxtype-avx512, whose worker
    /// has no Vulkan, under a config with cohere.gguf_backend = "vulkan".
    #[test]
    fn vulkan_backend_on_a_cpu_only_worker_is_a_mismatch() {
        let config = cohere_gguf("vulkan");
        let worker = std::path::Path::new("/usr/lib/voxtype/voxtype-gguf-worker-avx512");
        let m = detect_gguf_backend_mismatch(&config, worker).expect("mismatch");
        assert_eq!(m.section, "cohere");
        assert_eq!(m.requested, "vulkan");
        assert_eq!(m.worker, "voxtype-gguf-worker-avx512");
        assert_eq!(
            m.summary(),
            "cohere.gguf_backend is \"vulkan\", but the GGUF worker of voxtype-avx512 \
             (voxtype-gguf-worker-avx512) runs on the CPU only, so the cohere model cannot load."
        );
        let fix = m.fix_with(&SWITCH);
        assert!(
            fix.contains("run `sudo voxtype setup gpu --enable`"),
            "{fix}"
        );
        assert!(
            fix.contains("`voxtype config set cohere.gguf_backend auto`"),
            "{fix}"
        );
    }

    #[test]
    fn vulkan_backend_on_a_vulkan_worker_is_fine() {
        let config = cohere_gguf("Vulkan");
        for worker in [
            "/usr/lib/voxtype/voxtype-gguf-worker-vulkan",
            "/usr/lib/voxtype/voxtype-gguf-worker-onnx-avx512",
            "/usr/lib/voxtype/migraphx/voxtype-gguf-worker-onnx-migraphx",
        ] {
            assert_eq!(
                detect_gguf_backend_mismatch(&config, std::path::Path::new(worker)),
                None,
                "{worker}"
            );
        }
    }

    #[test]
    fn cpu_and_auto_backends_never_mismatch() {
        let worker = std::path::Path::new("/usr/lib/voxtype/voxtype-gguf-worker-avx2");
        for backend in ["auto", "cpu", "cpu_accel"] {
            assert_eq!(
                detect_gguf_backend_mismatch(&cohere_gguf(backend), worker),
                None,
                "{backend}"
            );
        }
    }

    #[test]
    fn unknown_workers_are_trusted() {
        // Source builds, VOXTYPE_GGUF_WORKER overrides and a generic worker
        // outside the package directory have whatever features they were
        // built with.
        let config = cohere_gguf("vulkan");
        for worker in [
            "/home/user/voxtype/target/release/voxtype-gguf-worker",
            "/opt/my-worker",
        ] {
            assert_eq!(
                detect_gguf_backend_mismatch(&config, std::path::Path::new(worker)),
                None,
                "{worker}"
            );
        }
    }

    /// Packages install the baseline build's worker under the generic name,
    /// and it has no GPU backend, so baseline with `vulkan` must be caught.
    #[test]
    fn packaged_generic_worker_is_the_cpu_only_baseline_worker() {
        let worker = std::path::Path::new("/usr/lib/voxtype/voxtype-gguf-worker");
        let mismatch = detect_gguf_backend_mismatch(&cohere_gguf("vulkan"), worker)
            .expect("baseline's worker has no Vulkan");
        assert!(
            mismatch.explanation().contains("voxtype-baseline"),
            "{}",
            mismatch.explanation()
        );
        assert_eq!(
            detect_gguf_backend_mismatch(&cohere_gguf("auto"), worker),
            None
        );
    }

    #[test]
    fn cuda_backend_points_at_vulkan() {
        let config = cohere_gguf("cuda");
        let worker =
            std::path::Path::new("/usr/lib/voxtype/cuda-13/voxtype-gguf-worker-onnx-cuda-13");
        let m = detect_gguf_backend_mismatch(&config, worker).expect("mismatch");
        assert!(
            m.summary().contains("was built without CUDA"),
            "{}",
            m.summary()
        );
        let fix = m.fix_with(&SWITCH);
        assert!(
            fix.contains("`voxtype config set cohere.gguf_backend vulkan`"),
            "{fix}"
        );
        assert!(!fix.contains("setup gpu --enable"), "{fix}");
    }

    #[test]
    fn a_non_gguf_model_is_not_checked() {
        let mut config = cohere_gguf("vulkan");
        config.cohere.as_mut().unwrap().model = "cohere-transcribe-q4f16".to_string();
        let worker = std::path::Path::new("/usr/lib/voxtype/voxtype-gguf-worker-avx512");
        assert_eq!(detect_gguf_backend_mismatch(&config, worker), None);
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
