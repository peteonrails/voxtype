# Baseline v2 Floor (Behavioral)

Prove the `baseline` release asset actually runs on an x86-64-v2 CPU by
executing real inference on one, not by reading the disassembly.

**Why this test is behavioral.** #740: a baseline build whose ggml half
escaped the toolchain constraints passed `--version`, passed `setup check`,
and loaded the model - all Rust startup paths - then SIGILLed on a BMI2
instruction at the first ggml matmul. No static instruction count can gate
this either: a correct build legitimately carries ~370 BMI2 (ring's
CPUID-dispatched assembly) and ~1800 FMA (rustfft's runtime kernels), and
the contamination added only ~60 BMI2 on top. Only executing inference on a
CPU without the instructions tells the truth. QEMU TCG cannot decode
instructions outside the modeled CPU, so `-cpu Nehalem` (SSE4.2 + POPCNT,
no AVX: the v2 floor) is a faithful stand-in for 2008-2013 hardware.

CI runs this on every baseline artifact (build-linux.yml, "Run inference on
a QEMU-modeled v2 CPU"). Run it by hand against the actual release asset
before a stable tag, and on the voxtype-ivybridge VM (CLAUDE.local.md) when
kernel-honest verification is wanted.

```bash
VERSION=1.1.0
WORK=$(mktemp -d)
cd "$WORK"

# The real release asset, checksum-verified - never a local build
gh release download "v${VERSION}" --repo peteonrails/voxtype \
  -p "voxtype-${VERSION}-linux-x86_64-baseline" -p 'SHA256SUMS.txt'
grep "x86_64-baseline$" SHA256SUMS.txt | sha256sum -c -
chmod +x "voxtype-${VERSION}-linux-x86_64-baseline"

# Isolated config + a small model + a spoken fixture
export XDG_CONFIG_HOME="$WORK/config" XDG_DATA_HOME="$WORK/data"
mkdir -p "$XDG_CONFIG_HOME/voxtype" "$XDG_DATA_HOME/voxtype/models"
printf 'engine = "whisper"\n\n[whisper]\nmodel = "tiny.en"\n' \
  > "$XDG_CONFIG_HOME/voxtype/config.toml"
curl -sSL -o "$XDG_DATA_HOME/voxtype/models/ggml-tiny.en.bin" \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin

# The gate: full inference under a v2-modeled CPU. Slow under TCG
# (roughly half a minute for a one-second clip); a SIGILL here means the
# binary would crash on real pre-Haswell hardware.
qemu-x86_64 -cpu Nehalem "./voxtype-${VERSION}-linux-x86_64-baseline" \
  transcribe <repo>/tests/fixtures/vad/speech_hello.wav
```

**Pass:** exit 0 and a transcription printed. **Fail:** SIGILL (the banner
in the baseline build says it is a build bug and asks for the CPU model).

The known-broken v1.1.0-rc4 asset reproducibly fails this test; use it to
verify the harness if in doubt.
