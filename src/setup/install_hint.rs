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

use std::cell::OnceCell;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::model_upgrade::{detect_install_method, CommandRunner, InstallMethod, SystemRunner};
use crate::setup::binary::{self, EngineFamily, Variant};

/// The record `voxtype-bin` writes beside its binaries.
pub const PACKAGE_VARIANTS_FILE: &str = "package-variants";

/// `install_method` value written by the AUR `voxtype-bin` package.
const AUR_BIN: &str = "aur-bin";

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

/// What the hint can see of this install. Probes that run a program are
/// deferred and done at most once, because most hints never reach them.
pub struct Host<'a> {
    /// The running binary is a packaged variant under `lib_dir`.
    running_from_package: bool,
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
        let canonical = exe.canonicalize().unwrap_or_else(|_| exe.clone());
        let running_from_package = canonical.starts_with(&lib_dir);
        let record = PackageRecord::read(&lib_dir.join(PACKAGE_VARIANTS_FILE));
        Self {
            running_from_package,
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
        let packaged = match need {
            Need::Feature(_) => self.running_from_package,
            // Switching variants acts on the package, whichever binary asks.
            Need::Variant(_) => self.running_from_package || self.lib_dir.is_dir(),
        };
        if !packaged {
            let feature = match need {
                // Engine names double as feature names except OpenVINO's.
                Need::Feature("openvino") => Some("openvino-whisper"),
                Need::Feature(feature) => Some(feature),
                Need::Variant(variant) => variant_feature(variant),
            };
            return Hint::Rebuild { feature };
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
    let pick = binary::recommend(&binary::detect_cpu(), &binary::detect_gpus()).onnx;
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

    #[test]
    fn the_install_method_is_probed_once() {
        let fx = Fixture::new(Some(Variant::WhisperAvx2), &[Variant::WhisperAvx2], None);
        let runner = runner(&[("pacman", "voxtype-bin\n")]);
        let host = fx.host(&runner);
        host.hint(Need::Feature("parakeet"));
        host.hint(Need::Feature("moonshine"));
        assert_eq!(runner.calls.borrow().len(), 1, "{:?}", runner.calls);
    }
}
