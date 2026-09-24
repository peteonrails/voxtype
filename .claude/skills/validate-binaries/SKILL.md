---
name: validate-binaries
description: Validate voxtype binaries for CPU instruction contamination. Use when checking release binaries for AVX-512/GFNI leaks (crash on pre-Icelake/Zen 3) or BMI2/v3 leaks in the baseline binary (crash on pre-Haswell).
user-invocable: true
allowed-tools:
  - Bash
  - Read
  - Glob
---

# Validate Binaries

Verify that voxtype binaries don't contain forbidden CPU instructions that would cause crashes on older CPUs.

## What This Checks

| Binary | Must NOT have | Must have |
|--------|---------------|-----------|
| baseline | zmm, AVX-512 EVEX, GFNI; must pass QEMU v2 inference | - |
| AVX2 | zmm registers, AVX-512 EVEX, GFNI | - |
| Vulkan | zmm registers, AVX-512 EVEX, GFNI | - |
| AVX-512 | - | zmm registers (confirms optimization) |

**The baseline binary has a lower floor (x86-64-v2) and its own failure
class.** #740: a baseline build whose ggml half escaped the toolchain file
passed every AVX-512 check, loaded models fine, and SIGILLed on `shrx` at
the first inference on real Ivy Bridge.

**No static count can gate the v2 floor.** A correct baseline binary
legitimately contains ~370 BMI2 instructions (ring's CPUID-dispatched
assembly: bn_mulx4x_mont, the chacha20 AVX2 path, curve25519 ADX) and ~1800
FMA (rustfft's runtime-dispatched AVX kernels). #740's ggml contamination
added only ~60 BMI2 on top of ring's baseline - inside the noise. Treat the
counts as a drift alarm.

**The decisive test is behavioral: real inference on a v2-modeled CPU.** CI
does this under `qemu-x86_64-static -cpu Nehalem` (see build-linux.yml) -
TCG genuinely cannot decode out-of-floor instructions, and the rc4 binary
reproducibly SIGILLs there while a correct build transcribes. Locally, the
voxtype-ivybridge VM in CLAUDE.local.md tests the true hardware path. Either
way the test must include `transcribe <wav>`, not just `--version` or
`setup check`: model load succeeds on a contaminated build, and only the
first ggml matmul executes the bad code.

## Forbidden Instructions

- `zmm` registers - 512-bit AVX-512 registers
- `vpternlog`, `vpermt2`, `vpblendm` - AVX-512 specific
- `{1to4}`, `{1to8}`, `{1to16}` - AVX-512 broadcast syntax
- `vgf2p8`, `gf2p8` - GFNI instructions (not on Zen 3)

## Usage

When asked to validate binaries:

1. Determine the version from context or ask
2. Find binaries in `releases/${VERSION}/`
3. Run objdump checks on each binary
4. Report pass/fail for each

## Validation Commands

```bash
# Set version
VERSION=0.4.14

# Check AVX2 binary (should be 0 for all)
echo "=== AVX2 Binary ==="
objdump -d releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-avx2 | grep -c zmm || echo "zmm: 0"
objdump -d releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-avx2 | grep -cE 'vpternlog|vpermt2|vpblendm' || echo "AVX-512 ops: 0"
objdump -d releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-avx2 | grep -cE 'vgf2p8|gf2p8' || echo "GFNI: 0"

# Check Vulkan binary (should be 0 for all)
echo "=== Vulkan Binary ==="
objdump -d releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-vulkan | grep -c zmm || echo "zmm: 0"
objdump -d releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-vulkan | grep -cE 'vgf2p8|gf2p8' || echo "GFNI: 0"

# Check AVX-512 binary (should be > 0)
echo "=== AVX-512 Binary ==="
objdump -d releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-avx512 | grep -c zmm

# Check baseline binary (zmm must be 0; BMI2/FMA are drift alarms, see #740:
# ~370 BMI2 from ring and ~1800 FMA from rustfft are expected and safe)
echo "=== Baseline Binary ==="
objdump -d releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-baseline | grep -c zmm || echo "zmm: 0"
objdump -d releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-baseline | grep -cP '\t(shrx|sarx|shlx|mulx|pdep|pext|bzhi|rorx)[ \t]' || echo "BMI2: 0"

# Behavioral check on a v2-modeled CPU (the only conclusive test):
# REAL INFERENCE, not just --version - model load succeeds on a broken build.
qemu-x86_64-static -cpu Nehalem releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-baseline transcribe tests/fixtures/vad/speech_hello.wav
```

## Interpreting Results

**PASS conditions:**
- AVX2: All counts are 0
- Vulkan: All counts are 0
- AVX-512: zmm count > 0

**FAIL conditions:**
- AVX2 or Vulkan has any zmm/GFNI instructions
- AVX-512 has 0 zmm instructions (not optimized)

If validation fails, the Docker build cache is likely stale. Recommend:
```bash
docker compose -f docker-compose.build.yml build --no-cache avx2 vulkan
```
