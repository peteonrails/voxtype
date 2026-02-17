---
name: build-release
description: Build all voxtype binaries for release. Builds Whisper (AVX2, AVX-512, Vulkan) and ONNX (AVX2, AVX-512, CUDA) binaries using Docker. Use when preparing a new release.
user-invocable: true
allowed-tools:
  - Bash
  - Read
  - Glob
---

# Build Release Binaries

Build all 6 voxtype binaries for a release:
- **Whisper**: AVX2, AVX-512, Vulkan
- **ONNX** (Parakeet + Moonshine): AVX2, AVX-512, CUDA

## Prerequisites

- Docker with remote context `truenas` configured (pre-AVX-512 server)
- Local AVX-512 capable CPU for AVX-512 builds
- Current branch pushed to origin

## Quick Reference

```bash
# Set version
export VERSION=X.Y.Z

# Build remote binaries (AVX2, Vulkan, ONNX-AVX2, ONNX-CUDA)
docker context use truenas
docker compose -f docker-compose.build.yml build --no-cache avx2 vulkan onnx-avx2 onnx-cuda
docker compose -f docker-compose.build.yml up avx2 vulkan onnx-avx2 onnx-cuda

# Build local AVX-512 binaries
docker context use default
cargo clean && cargo build --release
cp target/release/voxtype releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-avx512
cargo clean && RUSTFLAGS="-C target-cpu=native" cargo build --release --features parakeet,moonshine
cp target/release/voxtype releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-onnx-avx512

# Verify versions
for bin in releases/${VERSION}/voxtype-*; do echo "$(basename $bin): $($bin --version)"; done

# Generate checksums
cd releases/${VERSION} && sha256sum voxtype-* > SHA256SUMS

# Create pre-release
gh release create v${VERSION} --prerelease --target BRANCH releases/${VERSION}/*
```

## Suggested Bash Permissions

To avoid prompts during the build process, add these to your allowed commands:

```
- tool: Bash
  prompt: switch docker context
- tool: Bash
  prompt: build docker images
- tool: Bash
  prompt: run docker containers
- tool: Bash
  prompt: copy docker container files
- tool: Bash
  prompt: cargo build
- tool: Bash
  prompt: copy binaries
- tool: Bash
  prompt: verify binary versions
- tool: Bash
  prompt: generate checksums
- tool: Bash
  prompt: create github release
- tool: Bash
  prompt: git push
```

## Workflow

### 1. Prepare Version

Ensure Cargo.toml is updated and committed:
```bash
# Edit Cargo.toml with new version
cargo build  # Updates Cargo.lock
git add Cargo.toml Cargo.lock
git commit -S -m "Bump to vX.Y.Z"
git push
```

### 2. Build Remote Binaries (AVX2, Vulkan, ONNX)

These builds use Ubuntu 22.04 to avoid AVX-512 instruction contamination:

```bash
export VERSION=X.Y.Z
docker context use truenas
mkdir -p releases/${VERSION}

# Build all Docker images (takes ~10-15 min)
docker compose -f docker-compose.build.yml build --no-cache avx2 vulkan onnx-avx2 onnx-cuda

# Extract binaries
docker compose -f docker-compose.build.yml up avx2 vulkan onnx-avx2 onnx-cuda
```

### 3. Build Local AVX-512 Binaries

AVX-512 builds require a host CPU with AVX-512 support:

```bash
docker context use default

# Whisper AVX-512
cargo clean && cargo build --release
cp target/release/voxtype releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-avx512

# ONNX AVX-512
cargo clean && RUSTFLAGS="-C target-cpu=native" cargo build --release --features parakeet,moonshine
cp target/release/voxtype releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-onnx-avx512
```

### 4. Verify All Binaries

```bash
for bin in releases/${VERSION}/voxtype-*; do
  echo -n "$(basename $bin): "
  $bin --version
done
```

All binaries must report the same version. If any differ, the Docker cache is stale.

### 5. Generate Checksums

```bash
cd releases/${VERSION}
sha256sum voxtype-* > SHA256SUMS
cat SHA256SUMS
```

### 6. Create GitHub Release

```bash
gh release create v${VERSION} \
  --title "vX.Y.Z: Title" \
  --prerelease \
  --target BRANCH \
  releases/${VERSION}/*
```

## Troubleshooting

### Wrong binary version

Docker caches aggressively. Rebuild with `--no-cache`:
```bash
docker compose -f docker-compose.build.yml build --no-cache avx2 vulkan
```

### Remote build output missing

If `docker compose up` shows old binaries, the containers are stale. Use `docker run` to run fresh containers from the new images.

### AVX-512 contamination in AVX2 binary

Check with objdump:
```bash
objdump -d releases/${VERSION}/voxtype-*-avx2 | grep -c zmm
```

Should be 0. If not, rebuild on a pre-AVX-512 host.

## Expected Output

After successful build, `releases/${VERSION}/` should contain:
```
voxtype-X.Y.Z-linux-x86_64-avx2
voxtype-X.Y.Z-linux-x86_64-avx512
voxtype-X.Y.Z-linux-x86_64-vulkan
voxtype-X.Y.Z-linux-x86_64-onnx-avx2
voxtype-X.Y.Z-linux-x86_64-onnx-avx512
voxtype-X.Y.Z-linux-x86_64-onnx-cuda
SHA256SUMS
```
