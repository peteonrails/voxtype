//! Package-aware remediation lines.
//!
//! When the running binary cannot do something (an engine it was built
//! without, a backend whose binary is missing), the fix depends on how voxtype
//! was installed. A source build has to be rebuilt with a Cargo feature. A
//! package install usually has the right variant on disk already and only
//! needs a `setup ... --enable`. An AUR `voxtype-bin` built for this machine
//! may have left the variant out, and a deb or rpm with a missing file needs a
//! reinstall. Every surface that suggests a fix asks [`Host::hint`] so they
//! all give the same advice.
//!
//! `voxtype-bin` records what it installed in `/usr/lib/voxtype/package-variants`
//! (see the AUR PKGBUILD). Deb and rpm do not write it, which means every
//! variant is installed. The record only changes the wording of a hint; nothing
//! requires it.
//!
//! A Nix install has neither `/usr/lib/voxtype` nor anything to rebuild in
//! place: each flake package is one fixed build in the read-only store, so
//! the fix there is to install the flake package that has what is missing.

use std::cell::OnceCell;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::model_upgrade::{detect_install_method, CommandRunner, InstallMethod, SystemRunner};
use crate::setup::binary::{self, EngineFamily, Variant};

/// The record `voxtype-bin` writes beside its binaries.
pub const PACKAGE_VARIANTS_FILE: &str = "package-variants";

/// `install_method` value written by the AUR `voxtype-bin` package.
const AUR_BIN: &str = "aur-bin";

/// Where every Nix-built binary lives.
const NIX_STORE: &str = "/nix/store";

/// True when `exe` is a Nix build. A Nix wrapper `exec`s the real binary, so
/// `/proc/self/exe` (and its canonical form) points into the store.
pub fn is_nix_store_path(exe: &Path) -> bool {
    exe.starts_with(NIX_STORE)
}

/// True when the running binary came from the Nix store.
pub fn running_from_nix() -> bool {
    let exe = binary::current_binary_path();
    is_nix_store_path(&exe.canonicalize().unwrap_or(exe))
}

/// What the fix has to provide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// A Cargo feature the running binary was built without, e.g. `parakeet`.
    /// Any installed variant that has it will do.
    Feature(&'static str),
    /// One specific packaged variant, e.g. the Vulkan binary for
    /// `setup gpu --enable`.
    Variant(Variant),
}

/// One remediation, rendered by `Display` as a clause that follows "to fix
/// this, ..." or "not available - ...".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hint {
    /// A variant that provides it is installed; switch to it.
    Switch { command: &'static str },
    /// `voxtype-bin` was built without the variant (`token` is its name in
    /// `VOXTYPE_VARIANTS`, e.g. `onnx-avx512`).
    AurVariant { token: String },
    /// A package install with the file missing.
    Reinstall { method: InstallMethod },
    /// A source build: rebuild with the feature.
    Rebuild { feature: Option<&'static str> },
    /// A Nix install: install the voxtype flake package (`package`, a
    /// `packages.<system>` output) that has it. `None` when no flake package
    /// includes `feature`.
    Nix {
        package: Option<&'static str>,
        feature: Option<&'static str>,
    },
}

impl fmt::Display for Hint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Hint::Switch { command } => write!(f, "run `{command}`"),
            Hint::AurVariant { token } => write!(
                f,
                "rebuild voxtype-bin with the {token} variant, which this build left out: \
                 `VOXTYPE_VARIANTS={token} yay -S --rebuild voxtype-bin` \
                 (or `VOXTYPE_VARIANTS={token} makepkg -si` in a clone of the AUR repo)"
            ),
            Hint::Reinstall { method } => match method {
                InstallMethod::Pacman(package) => write!(
                    f,
                    "reinstall the {package} package (`sudo pacman -S {package}`, \
                     or your AUR helper)"
                ),
                InstallMethod::Dpkg(package) => write!(
                    f,
                    "reinstall the {package} package (`sudo apt install --reinstall {package}`)"
                ),
                InstallMethod::Rpm(package) => write!(
                    f,
                    "reinstall the {package} package (`sudo dnf reinstall {package}`)"
                ),
                InstallMethod::Nix | InstallMethod::Other => {
                    write!(f, "reinstall the voxtype package")
                }
            },
            Hint::Rebuild {
                feature: Some(feature),
            } => write!(f, "rebuild with --features {feature}"),
            Hint::Rebuild { feature: None } => write!(f, "rebuild voxtype"),
            Hint::Nix {
                package: Some(package),
                ..
            } => write!(
                f,
                "install the voxtype flake's `{package}` package: set \
                 `programs.voxtype.package = voxtype.packages.${{system}}.{package};` \
                 in your NixOS or Home Manager configuration, or run \
                 `nix profile install github:peteonrails/voxtype#{package}`"
            ),
            Hint::Nix {
                package: None,
                feature: Some(feature),
            } => write!(
                f,
                "build voxtype from source with --features {feature}; \
                 no voxtype flake package includes it"
            ),
            Hint::Nix {
                package: None,
                feature: None,
            } => write!(f, "build voxtype from source"),
        }
    }
}

/// `/usr/lib/voxtype/package-variants`, as far as hints care.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageRecord {
    pub install_method: String,
    pub included: Vec<String>,
}

impl PackageRecord {
    /// `key=value` lines; `#` comments, blank lines and unknown keys ignored.
    pub fn parse(text: &str) -> Self {
        let mut record = Self::default();
        for line in text.lines().map(str::trim) {
            if line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "install_method" => record.install_method = value.trim().to_string(),
                "included" => {
                    record.included = value.split_whitespace().map(str::to_string).collect()
                }
                _ => {}
            }
        }
        record
    }

    pub fn read(path: &Path) -> Option<Self> {
        std::fs::read_to_string(path)
            .ok()
            .map(|text| Self::parse(&text))
    }

    fn is_aur_bin(&self) -> bool {
        self.install_method == AUR_BIN
    }

    fn includes(&self, token: &str) -> bool {
        self.included.iter().any(|t| t == token)
    }
}

/// A variant's name in `VOXTYPE_VARIANTS` and the package record.
pub fn variant_token(variant: Variant) -> &'static str {
    let name = variant.binary_name();
    name.strip_prefix("voxtype-").unwrap_or(name)
}

/// The command that switches `/usr/bin/voxtype` to an installed variant.
fn switch_command(variant: Variant) -> &'static str {
    match variant.family() {
        EngineFamily::Onnx => "sudo voxtype setup onnx --enable",
        EngineFamily::Whisper if variant == Variant::WhisperVulkan => {
            "sudo voxtype setup gpu --enable"
        }
        EngineFamily::Whisper => "sudo voxtype setup gpu --disable",
    }
}

/// Which packaged variants provide a Cargo feature. Every release binary
/// carries the GGUF client features, so those match any variant.
fn provides(variant: Variant, feature: &str) -> bool {
    match feature {
        "gguf" | "cohere-gguf" | "parakeet-gguf" => true,
        "gpu-vulkan" => variant == Variant::WhisperVulkan,
        "openvino-whisper" | "ml-diarization" => variant.family() == EngineFamily::Onnx,
        engine => variant.family() == EngineFamily::Onnx && variant.supports_engine(engine),
    }
}

/// The Cargo feature a source build needs to produce a variant.
fn variant_feature(variant: Variant) -> Option<&'static str> {
    match variant {
        Variant::WhisperVulkan => Some("gpu-vulkan"),
        Variant::OnnxCuda | Variant::OnnxCuda12 | Variant::OnnxCuda13 => Some("parakeet-cuda"),
        Variant::OnnxMigraphx => Some("parakeet-migraphx"),
        Variant::OnnxAvx2 | Variant::OnnxAvx512 | Variant::OnnxNative => Some("parakeet"),
        Variant::WhisperBaseline
        | Variant::WhisperAvx2
        | Variant::WhisperAvx512
        | Variant::WhisperNative => None,
    }
}

/// The Cargo feature a build needs to provide what `need` asks for.
fn build_feature(need: Need) -> Option<&'static str> {
    match need {
        // Engine names double as feature names except OpenVINO's.
        Need::Feature("openvino") => Some("openvino-whisper"),
        Need::Feature(feature) => Some(feature),
        Need::Variant(variant) => variant_feature(variant),
    }
}

/// The voxtype flake package (a `packages.<system>` output in flake.nix)
/// that is the Nix counterpart of a packaged variant.
pub fn nix_package_for_variant(variant: Variant) -> &'static str {
    match variant {
        Variant::WhisperVulkan => "vulkan",
        Variant::OnnxCuda | Variant::OnnxCuda12 | Variant::OnnxCuda13 => "onnx-cuda",
        Variant::OnnxMigraphx => "onnx-migraphx",
        Variant::OnnxAvx2 | Variant::OnnxAvx512 | Variant::OnnxNative => "onnx",
        Variant::WhisperBaseline
        | Variant::WhisperAvx2
        | Variant::WhisperAvx512
        | Variant::WhisperNative => "default",
    }
}

/// The flake package that provides what `need` asks for, if any does.
/// Every flake package carries the GGUF clients; none carries OpenVINO or
/// ML diarization.
fn nix_package(need: Need) -> Option<&'static str> {
    match need {
        Need::Variant(variant) => Some(nix_package_for_variant(variant)),
        Need::Feature("gguf" | "cohere-gguf" | "parakeet-gguf") => Some("default"),
        Need::Feature("gpu-vulkan") => Some("vulkan"),
        Need::Feature("gpu-hipblas") => Some("rocm"),
        // The release ONNX variants carry these; the flake's do not.
        Need::Feature("openvino" | "openvino-whisper" | "ml-diarization") => None,
        Need::Feature(engine) if provides(Variant::OnnxAvx2, engine) => Some("onnx"),
        Need::Feature(_) => None,
    }
}

/// The flake package whose build has `feature`, if any.
pub fn nix_package_for_feature(feature: &'static str) -> Option<&'static str> {
    nix_package(Need::Feature(feature))
}

/// The flake package this binary was built as, read from its features.
pub fn running_nix_package() -> &'static str {
    if cfg!(feature = "parakeet-cuda") {
        "onnx-cuda"
    } else if cfg!(feature = "parakeet-migraphx") {
        "onnx-migraphx"
    } else if cfg!(feature = "parakeet") {
        "onnx"
    } else if cfg!(feature = "gpu-vulkan") {
        "vulkan"
    } else if cfg!(feature = "gpu-hipblas") {
        "rocm"
    } else {
        "default"
    }
}

/// The flake packages, with what each one runs, for status output.
const NIX_PACKAGES: &[(&str, &str)] = &[
    ("default", "Whisper on the CPU"),
    ("vulkan", "Whisper on a Vulkan GPU (AMD, Intel, NVIDIA)"),
    ("rocm", "Whisper on an AMD GPU through ROCm"),
    (
        "onnx",
        "ONNX engines (Parakeet, Moonshine, SenseVoice, ...) on the CPU",
    ),
    ("onnx-cuda", "ONNX engines on an NVIDIA GPU"),
    ("onnx-migraphx", "ONNX engines on an AMD GPU (MIGraphX)"),
];

/// What `setup gpu` and `setup onnx` print in place of the package status
/// on a Nix install, where there are no variants to list or switch.
pub fn nix_status_lines(exe: &Path, running: &str) -> Vec<String> {
    let mut lines = vec![
        format!("Installed with Nix: {}", exe.display()),
        format!("  Flake package: {running}"),
        String::new(),
        "Each voxtype flake package is one fixed build, so `setup gpu` and".to_string(),
        "`setup onnx` have nothing to switch. Choose the build by installing".to_string(),
        "its package (programs.voxtype.package, or nix profile install):".to_string(),
    ];
    for (package, runs) in NIX_PACKAGES {
        let mark = if *package == running { "*" } else { " " };
        lines.push(format!("  {mark} {package:<14} {runs}"));
    }
    lines
}

/// The error for a `setup` command that swaps packaged binaries, run on a
/// Nix install. `command` is what was run, e.g. `setup gpu --enable`;
/// `target` is the variant that command would have switched to and
/// `running` the flake package this binary was built as.
pub fn nix_switch_refusal(command: &str, target: Variant, running: &str) -> String {
    let package = nix_package_for_variant(target);
    let runs = NIX_PACKAGES
        .iter()
        .find(|(name, _)| *name == package)
        .map_or(package, |(_, runs)| *runs);
    let fix = if package == running {
        format!("This is already the `{package}` flake package ({runs}).")
    } else {
        let hint = Hint::Nix {
            package: Some(package),
            feature: None,
        };
        format!("For {runs}, {hint}.")
    };
    format!(
        "`voxtype {command}` switches between the binaries a deb, rpm or AUR package \
         installs in /usr/lib/voxtype.\n\
         This voxtype was installed with Nix, where each flake package is one fixed \
         build, so there is nothing to switch and nothing was changed.\n\
         {fix}"
    )
}

/// The build `setup gpu --enable` (`enable`) or `--disable` would move a
/// Nix install to. `onnx_build` says this binary runs the ONNX engines;
/// `onnx_pick` is the ONNX variant for this machine's hardware.
fn nix_gpu_target_for(enable: bool, onnx_build: bool, onnx_pick: Variant) -> Variant {
    let onnx_gpu = matches!(
        onnx_pick,
        Variant::OnnxCuda | Variant::OnnxCuda12 | Variant::OnnxCuda13 | Variant::OnnxMigraphx
    );
    match (enable, onnx_build) {
        (true, true) if onnx_gpu => onnx_pick,
        (true, _) => Variant::WhisperVulkan,
        (false, true) => Variant::OnnxAvx2,
        (false, false) => Variant::WhisperAvx2,
    }
}

/// [`nix_gpu_target_for`] on this machine.
pub fn nix_gpu_target(enable: bool) -> Variant {
    let onnx_build = cfg!(feature = "parakeet");
    let pick = if enable && onnx_build {
        system_onnx_pick()
    } else {
        Variant::OnnxAvx2
    };
    nix_gpu_target_for(enable, onnx_build, pick)
}

/// The ONNX build for this machine's hardware, as `setup onnx --enable`
/// would pick it.
pub fn nix_onnx_target() -> Variant {
    system_onnx_pick()
}

/// What the hint can see of this install. Probes that run a program are
/// deferred and done at most once, because most hints never reach them.
pub struct Host<'a> {
    /// The running binary is a packaged variant under `lib_dir`.
    running_from_package: bool,
    /// The running binary is a Nix build.
    nix: bool,
    exe: PathBuf,
    lib_dir: PathBuf,
    record: Option<PackageRecord>,
    runner: &'a dyn CommandRunner,
    /// The ONNX variant to suggest adding for this machine's hardware.
    onnx_pick: fn() -> Variant,
    method: OnceCell<InstallMethod>,
    onnx: OnceCell<Variant>,
}

impl<'a> Host<'a> {
    pub fn new(
        exe: PathBuf,
        lib_dir: PathBuf,
        runner: &'a dyn CommandRunner,
        onnx_pick: fn() -> Variant,
    ) -> Self {
        // Both sides canonical, or a symlink in the package path (macOS's
        // /var -> /private/var, a symlinked /usr/lib) puts the resolved
        // executable outside the unresolved directory and every package
        // install is advised to rebuild from source.
        let canonical = exe.canonicalize().unwrap_or_else(|_| exe.clone());
        let lib_dir = lib_dir.canonicalize().unwrap_or(lib_dir);
        let running_from_package = canonical.starts_with(&lib_dir);
        let nix = is_nix_store_path(&canonical);
        let record = PackageRecord::read(&lib_dir.join(PACKAGE_VARIANTS_FILE));
        Self {
            running_from_package,
            nix,
            exe: canonical,
            lib_dir,
            record,
            runner,
            onnx_pick,
            method: OnceCell::new(),
            onnx: OnceCell::new(),
        }
    }

    pub fn hint(&self, need: Need) -> Hint {
        if self.nix {
            return Hint::Nix {
                package: nix_package(need),
                feature: build_feature(need),
            };
        }
        let packaged = match need {
            Need::Feature(_) => self.running_from_package,
            // Switching variants acts on the package, whichever binary asks.
            Need::Variant(_) => self.running_from_package || self.lib_dir.is_dir(),
        };
        if !packaged {
            return Hint::Rebuild {
                feature: build_feature(need),
            };
        }

        let installed = |variant: Variant| self.lib_dir.join(variant.binary_name()).exists();
        let present = match need {
            Need::Variant(variant) => installed(variant).then_some(variant),
            Need::Feature(feature) => Variant::ALL
                .iter()
                .copied()
                .find(|&variant| provides(variant, feature) && installed(variant)),
        };
        if let Some(variant) = present {
            return Hint::Switch {
                command: switch_command(variant),
            };
        }

        let target = match need {
            Need::Variant(variant) => Some(variant),
            Need::Feature("gpu-vulkan") => Some(Variant::WhisperVulkan),
            Need::Feature(feature) if provides(Variant::OnnxAvx2, feature) => {
                Some(*self.onnx.get_or_init(self.onnx_pick))
            }
            Need::Feature(_) => None,
        };
        if let (Some(record), Some(target)) = (&self.record, target) {
            let token = variant_token(target);
            if record.is_aur_bin() && !record.includes(token) {
                return Hint::AurVariant {
                    token: token.to_string(),
                };
            }
        }

        let method = self
            .method
            .get_or_init(|| detect_install_method(&self.exe, self.runner));
        Hint::Reinstall {
            method: method.clone(),
        }
    }
}

/// The ONNX variant to suggest for this machine, with an unversioned CUDA
/// pick resolved to the CUDA major the host runs (13 when unknown, as
/// `setup onnx --enable` assumes).
fn system_onnx_pick() -> Variant {
    // No ONNX build runs on a CPU without AVX2, so there is no right answer
    // there; keep naming the AVX2 build as before. `info variants` and
    // `setup onnx --enable` both tell that user why it will not run.
    let pick = binary::recommend(&binary::detect_cpu(), &binary::detect_gpus(), false)
        .onnx
        .unwrap_or(Variant::OnnxAvx2);
    if pick == Variant::OnnxCuda {
        match super::parakeet::detect_cuda_runtime_major() {
            Some(12) => Variant::OnnxCuda12,
            _ => Variant::OnnxCuda13,
        }
    } else {
        pick
    }
}

impl Host<'static> {
    /// This machine: the running executable and the package directory.
    pub fn system() -> Self {
        Host::new(
            binary::current_binary_path(),
            PathBuf::from(binary::LIB_DIR),
            &SystemRunner,
            system_onnx_pick,
        )
    }
}

/// The remediation for this machine, for one-off callers.
pub fn hint(need: Need) -> Hint {
    Host::system().hint(need)
}

/// How to get an ONNX variant onto this machine when none is installed,
/// naming the one that suits its hardware.
pub fn onnx_variant_hint() -> Hint {
    hint(Need::Variant(system_onnx_pick()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Answers from a table and records every call, so a test can assert no
    /// package manager ran.
    #[derive(Default)]
    struct FakeRunner {
        answers: HashMap<&'static str, &'static str>,
        calls: RefCell<Vec<String>>,
    }

    impl CommandRunner for FakeRunner {
        fn output(&self, program: &str, args: &[&str]) -> Option<String> {
            self.calls
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")));
            self.answers.get(program).map(|s| s.to_string())
        }
    }

    fn runner(answers: &[(&'static str, &'static str)]) -> FakeRunner {
        FakeRunner {
            answers: answers.iter().copied().collect(),
            calls: RefCell::default(),
        }
    }

    fn pick_avx512() -> Variant {
        Variant::OnnxAvx512
    }

    /// A fake `/usr/lib/voxtype` holding `variants`, with the running binary
    /// being `running` inside it (or outside it when `None`).
    struct Fixture {
        dir: tempfile::TempDir,
        exe: PathBuf,
    }

    impl Fixture {
        fn new(running: Option<Variant>, variants: &[Variant], record: Option<&str>) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let lib = dir.path().join("lib");
            std::fs::create_dir(&lib).unwrap();
            for variant in variants {
                std::fs::write(lib.join(variant.binary_name()), []).unwrap();
            }
            if let Some(record) = record {
                std::fs::write(lib.join(PACKAGE_VARIANTS_FILE), record).unwrap();
            }
            let exe = match running {
                Some(variant) => lib.join(variant.binary_name()),
                None => {
                    let exe = dir.path().join("target/release/voxtype");
                    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
                    std::fs::write(&exe, []).unwrap();
                    exe
                }
            };
            Self { dir, exe }
        }

        fn host<'a>(&self, runner: &'a FakeRunner) -> Host<'a> {
            Host::new(
                self.exe.clone(),
                self.dir.path().join("lib"),
                runner,
                pick_avx512,
            )
        }
    }

    const AUR_RECORD: &str = "\
# voxtype-bin 1.2.1-1: variants selected when the package was built
install_method=aur-bin
included=avx2 baseline vulkan
explicit=
";

    #[test]
    fn parses_the_package_record() {
        let record = PackageRecord::parse(
            "# voxtype-bin 1.2.1-1\ninstall_method=aur-bin\nincluded=avx2 baseline vulkan onnx-migraphx\nexplicit=onnx-migraphx\nfuture=1\n",
        );
        assert_eq!(record.install_method, "aur-bin");
        assert_eq!(
            record.included,
            ["avx2", "baseline", "vulkan", "onnx-migraphx"]
        );
        assert!(record.includes("onnx-migraphx"));
        assert!(!record.includes("onnx-avx2"));
    }

    #[test]
    fn tokens_match_the_aur_variant_names() {
        assert_eq!(variant_token(Variant::WhisperBaseline), "baseline");
        assert_eq!(variant_token(Variant::WhisperVulkan), "vulkan");
        assert_eq!(variant_token(Variant::OnnxCuda13), "onnx-cuda-13");
        assert_eq!(variant_token(Variant::OnnxMigraphx), "onnx-migraphx");
    }

    #[test]
    fn installed_onnx_variant_gives_the_enable_command() {
        let fx = Fixture::new(
            Some(Variant::WhisperAvx2),
            &[Variant::WhisperAvx2, Variant::OnnxAvx2],
            None,
        );
        let runner = runner(&[]);
        let hint = fx.host(&runner).hint(Need::Feature("parakeet"));
        assert_eq!(
            hint,
            Hint::Switch {
                command: "sudo voxtype setup onnx --enable"
            }
        );
        assert_eq!(hint.to_string(), "run `sudo voxtype setup onnx --enable`");
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn installed_vulkan_variant_gives_setup_gpu() {
        let fx = Fixture::new(
            Some(Variant::WhisperAvx2),
            &[Variant::WhisperAvx2, Variant::WhisperVulkan],
            Some(AUR_RECORD),
        );
        let runner = runner(&[]);
        let host = fx.host(&runner);
        assert_eq!(
            host.hint(Need::Variant(Variant::WhisperVulkan)).to_string(),
            "run `sudo voxtype setup gpu --enable`"
        );
        assert_eq!(
            host.hint(Need::Feature("gpu-vulkan")).to_string(),
            "run `sudo voxtype setup gpu --enable`"
        );
    }

    #[test]
    fn aur_build_without_the_variant_names_voxtype_variants() {
        let fx = Fixture::new(
            Some(Variant::WhisperAvx2),
            &[
                Variant::WhisperAvx2,
                Variant::WhisperBaseline,
                Variant::WhisperVulkan,
            ],
            Some(AUR_RECORD),
        );
        let runner = runner(&[("pacman", "voxtype-bin\n")]);
        let host = fx.host(&runner);

        let hint = host.hint(Need::Feature("moonshine"));
        assert_eq!(
            hint,
            Hint::AurVariant {
                token: "onnx-avx512".into()
            }
        );
        assert_eq!(
            hint.to_string(),
            "rebuild voxtype-bin with the onnx-avx512 variant, which this build left out: \
             `VOXTYPE_VARIANTS=onnx-avx512 yay -S --rebuild voxtype-bin` \
             (or `VOXTYPE_VARIANTS=onnx-avx512 makepkg -si` in a clone of the AUR repo)"
        );

        let hint = host.hint(Need::Variant(Variant::OnnxMigraphx));
        assert_eq!(
            hint,
            Hint::AurVariant {
                token: "onnx-migraphx".into()
            }
        );
        // The record answered; no package manager was asked.
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn aur_record_that_lists_a_missing_variant_falls_back_to_reinstall() {
        // Included in the record but gone from disk: a reinstall restores it.
        let record = "install_method=aur-bin\nincluded=avx2 onnx-avx512\n";
        let fx = Fixture::new(
            Some(Variant::WhisperAvx2),
            &[Variant::WhisperAvx2],
            Some(record),
        );
        let runner = runner(&[("pacman", "voxtype-bin\n")]);
        assert_eq!(
            fx.host(&runner).hint(Need::Feature("parakeet")).to_string(),
            "reinstall the voxtype-bin package (`sudo pacman -S voxtype-bin`, or your AUR helper)"
        );
    }

    #[test]
    fn deb_and_rpm_installs_say_reinstall() {
        let fx = Fixture::new(Some(Variant::WhisperAvx2), &[Variant::WhisperAvx2], None);

        let dpkg = runner(&[("dpkg", "voxtype: /usr/bin/voxtype\n")]);
        assert_eq!(
            fx.host(&dpkg).hint(Need::Feature("parakeet")).to_string(),
            "reinstall the voxtype package (`sudo apt install --reinstall voxtype`)"
        );

        let rpm = runner(&[("rpm", "voxtype")]);
        assert_eq!(
            fx.host(&rpm)
                .hint(Need::Variant(Variant::WhisperVulkan))
                .to_string(),
            "reinstall the voxtype package (`sudo dnf reinstall voxtype`)"
        );

        let unknown = runner(&[]);
        assert_eq!(
            fx.host(&unknown)
                .hint(Need::Feature("parakeet"))
                .to_string(),
            "reinstall the voxtype package"
        );
    }

    #[test]
    fn non_aur_record_is_ignored() {
        let record = "install_method=something-else\nincluded=avx2\n";
        let fx = Fixture::new(
            Some(Variant::WhisperAvx2),
            &[Variant::WhisperAvx2],
            Some(record),
        );
        let runner = runner(&[("dpkg", "voxtype: /usr/bin/voxtype\n")]);
        assert!(matches!(
            fx.host(&runner).hint(Need::Feature("parakeet")),
            Hint::Reinstall { .. }
        ));
    }

    #[test]
    fn source_build_says_rebuild_with_the_feature() {
        let fx = Fixture::new(None, &[], None);
        let runner = runner(&[("pacman", "voxtype-bin\n")]);
        let host = fx.host(&runner);
        let hint = host.hint(Need::Feature("sensevoice"));
        assert_eq!(hint.to_string(), "rebuild with --features sensevoice");
        assert_eq!(
            host.hint(Need::Feature("openvino")).to_string(),
            "rebuild with --features openvino-whisper"
        );
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn source_build_without_a_package_maps_variants_to_features() {
        let fx = Fixture::new(None, &[], None);
        std::fs::remove_dir(fx.dir.path().join("lib")).unwrap();
        let runner = runner(&[]);
        let host = fx.host(&runner);
        assert_eq!(
            host.hint(Need::Variant(Variant::WhisperVulkan)).to_string(),
            "rebuild with --features gpu-vulkan"
        );
        assert_eq!(
            host.hint(Need::Variant(Variant::OnnxMigraphx)).to_string(),
            "rebuild with --features parakeet-migraphx"
        );
        assert_eq!(
            host.hint(Need::Variant(Variant::WhisperAvx2)).to_string(),
            "rebuild voxtype"
        );
    }

    #[test]
    fn variant_switch_from_a_source_binary_still_reads_the_package() {
        // `setup gpu --enable` run through a source build next to a package
        // install acts on the package, so the package decides the hint.
        let fx = Fixture::new(None, &[Variant::WhisperVulkan], None);
        let runner = runner(&[]);
        assert_eq!(
            fx.host(&runner)
                .hint(Need::Variant(Variant::WhisperVulkan))
                .to_string(),
            "run `sudo voxtype setup gpu --enable`"
        );
    }

    /// The package directory reached through a symlink, as macOS's temp dir
    /// is (/var -> /private/var): the running binary still counts as the
    /// package's. Reproduces on Linux what failed every package case on macOS.
    #[cfg(unix)]
    #[test]
    fn package_dir_behind_a_symlink_is_still_the_package() {
        let fx = Fixture::new(Some(Variant::OnnxAvx2), &[Variant::OnnxAvx2], None);
        let link = fx.dir.path().join("link");
        std::os::unix::fs::symlink(fx.dir.path(), &link).unwrap();
        let runner = runner(&[]);
        let host = Host::new(
            link.join("lib").join(Variant::OnnxAvx2.binary_name()),
            link.join("lib"),
            &runner,
            pick_avx512,
        );
        assert_eq!(
            host.hint(Need::Feature("parakeet")),
            Hint::Switch {
                command: "sudo voxtype setup onnx --enable"
            }
        );
    }

    #[test]
    fn the_install_method_is_probed_once() {
        let fx = Fixture::new(Some(Variant::WhisperAvx2), &[Variant::WhisperAvx2], None);
        let runner = runner(&[("pacman", "voxtype-bin\n")]);
        let host = fx.host(&runner);
        host.hint(Need::Feature("parakeet"));
        host.hint(Need::Feature("moonshine"));
        assert_eq!(runner.calls.borrow().len(), 1, "{:?}", runner.calls);
    }

    fn nix_host(runner: &FakeRunner, lib_dir: PathBuf) -> Host<'_> {
        Host::new(
            PathBuf::from("/nix/store/abc123-voxtype-1.3.1/bin/voxtype"),
            lib_dir,
            runner,
            pick_avx512,
        )
    }

    #[test]
    fn nix_install_names_the_flake_package_not_a_rebuild() {
        let runner = runner(&[("pacman", "voxtype-bin\n")]);
        let host = nix_host(&runner, PathBuf::from("/nonexistent/usr/lib/voxtype"));

        let hint = host.hint(Need::Feature("parakeet"));
        assert_eq!(
            hint,
            Hint::Nix {
                package: Some("onnx"),
                feature: Some("parakeet"),
            }
        );
        assert_eq!(
            hint.to_string(),
            "install the voxtype flake's `onnx` package: set \
             `programs.voxtype.package = voxtype.packages.${system}.onnx;` \
             in your NixOS or Home Manager configuration, or run \
             `nix profile install github:peteonrails/voxtype#onnx`"
        );
        assert_eq!(
            host.hint(Need::Variant(Variant::WhisperVulkan)),
            Hint::Nix {
                package: Some("vulkan"),
                feature: Some("gpu-vulkan"),
            }
        );
        assert_eq!(
            host.hint(Need::Variant(Variant::OnnxCuda13)),
            Hint::Nix {
                package: Some("onnx-cuda"),
                feature: Some("parakeet-cuda"),
            }
        );
        assert_eq!(
            host.hint(Need::Feature("gpu-vulkan")).to_string(),
            host.hint(Need::Variant(Variant::WhisperVulkan)).to_string()
        );
        // No package manager is asked about a store path.
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn nix_install_without_a_flake_package_says_so() {
        let runner = runner(&[]);
        let host = nix_host(&runner, PathBuf::from("/nonexistent/usr/lib/voxtype"));
        for feature in ["openvino", "ml-diarization"] {
            let hint = host.hint(Need::Feature(feature));
            assert!(matches!(hint, Hint::Nix { package: None, .. }), "{hint:?}");
        }
        assert_eq!(
            host.hint(Need::Feature("openvino")).to_string(),
            "build voxtype from source with --features openvino-whisper; \
             no voxtype flake package includes it"
        );
    }

    #[test]
    fn nix_install_ignores_a_package_directory_beside_it() {
        // A leftover /usr/lib/voxtype does not make a Nix binary a package one.
        let fx = Fixture::new(None, &[Variant::OnnxAvx2, Variant::WhisperVulkan], None);
        let runner = runner(&[]);
        let host = nix_host(&runner, fx.dir.path().join("lib"));
        assert!(matches!(
            host.hint(Need::Feature("parakeet")),
            Hint::Nix {
                package: Some("onnx"),
                ..
            }
        ));
        assert!(matches!(
            host.hint(Need::Variant(Variant::WhisperVulkan)),
            Hint::Nix {
                package: Some("vulkan"),
                ..
            }
        ));
    }

    #[test]
    fn store_paths_are_nix_and_package_paths_are_not() {
        assert!(is_nix_store_path(Path::new(
            "/nix/store/abc-voxtype-1.3.1/bin/.voxtype-wrapped"
        )));
        assert!(!is_nix_store_path(Path::new(
            "/usr/lib/voxtype/voxtype-avx2"
        )));
        assert!(!is_nix_store_path(Path::new("/home/u/nix/store/voxtype")));
    }

    /// Every package a hint or the status names is a real flake output.
    #[test]
    fn nix_packages_named_exist_in_the_flake() {
        let flake = include_str!("../../flake.nix");
        let mut names: Vec<&str> = NIX_PACKAGES.iter().map(|(name, _)| *name).collect();
        names.extend(Variant::ALL.iter().map(|v| nix_package_for_variant(*v)));
        names.push(running_nix_package());
        for name in names {
            assert!(
                flake.contains(&format!("\n          {name} = ")),
                "flake.nix has no packages.{name}"
            );
        }
    }

    #[test]
    fn nix_switch_refusal_explains_and_names_the_package() {
        let message = nix_switch_refusal("setup gpu --enable", Variant::WhisperVulkan, "default");
        assert_eq!(
            message,
            "`voxtype setup gpu --enable` switches between the binaries a deb, rpm or AUR \
             package installs in /usr/lib/voxtype.\n\
             This voxtype was installed with Nix, where each flake package is one fixed \
             build, so there is nothing to switch and nothing was changed.\n\
             For Whisper on a Vulkan GPU (AMD, Intel, NVIDIA), install the voxtype flake's \
             `vulkan` package: set `programs.voxtype.package = voxtype.packages.${system}.vulkan;` \
             in your NixOS or Home Manager configuration, or run \
             `nix profile install github:peteonrails/voxtype#vulkan`."
        );

        let already = nix_switch_refusal("setup gpu --enable", Variant::WhisperVulkan, "vulkan");
        assert!(
            already.ends_with(
                "This is already the `vulkan` flake package \
                 (Whisper on a Vulkan GPU (AMD, Intel, NVIDIA))."
            ),
            "{already}"
        );

        let onnx = nix_switch_refusal("setup onnx --enable", Variant::OnnxCuda13, "default");
        assert!(
            onnx.contains("For ONNX engines on an NVIDIA GPU, install"),
            "{onnx}"
        );
        assert!(onnx.contains("voxtype#onnx-cuda`"), "{onnx}");
    }

    #[test]
    fn nix_status_marks_the_running_package() {
        let lines = nix_status_lines(
            Path::new("/nix/store/abc-voxtype-vulkan-1.3.1/bin/voxtype"),
            "vulkan",
        );
        assert_eq!(
            lines[0],
            "Installed with Nix: /nix/store/abc-voxtype-vulkan-1.3.1/bin/voxtype"
        );
        assert!(lines.iter().any(|l| l.starts_with("  * vulkan ")));
        assert!(lines.iter().any(|l| l.starts_with("    default ")));
        assert!(!lines.iter().any(|l| l.contains("/usr/lib/voxtype")));
    }

    #[test]
    fn nix_gpu_targets_follow_the_build_and_the_hardware() {
        use Variant::*;
        // Whisper builds move between default and vulkan.
        assert_eq!(nix_gpu_target_for(true, false, OnnxCuda13), WhisperVulkan);
        assert_eq!(nix_gpu_target_for(false, false, OnnxCuda13), WhisperAvx2);
        // ONNX builds move to the GPU ONNX package the hardware supports.
        assert_eq!(nix_gpu_target_for(true, true, OnnxCuda13), OnnxCuda13);
        assert_eq!(nix_gpu_target_for(true, true, OnnxMigraphx), OnnxMigraphx);
        assert_eq!(nix_gpu_target_for(false, true, OnnxCuda13), OnnxAvx2);
        // No GPU the ONNX engines can use: Vulkan is the GPU build there is.
        assert_eq!(nix_gpu_target_for(true, true, OnnxAvx512), WhisperVulkan);
        assert_eq!(nix_package_for_variant(OnnxCuda13), "onnx-cuda");
        assert_eq!(nix_package_for_variant(WhisperAvx2), "default");
    }
}
