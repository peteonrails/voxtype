# CI Build Setup for TrueNAS SCALE

This guide explains how to set up automated builds on TrueNAS SCALE (or any Docker-capable host) to produce voxtype binaries without AVX-512 instructions.

## Why Build on TrueNAS?

The i9-9900K (Coffee Lake) processor has AVX2 but **not** AVX-512. Building on this machine ensures:

- No AVX-512 instructions leak from auto-vectorization
- Binaries work on older CPUs (Zen 3, Haswell, etc.)
- Clean, reproducible builds

## Quick Start

1. Clone the repository on your TrueNAS SCALE machine:
   ```bash
   git clone https://github.com/peteonrails/voxtype.git
   cd voxtype
   ```

2. Build all binaries:
   ```bash
   ./scripts/ci-build.sh
   ```

3. Find binaries in `releases/<version>/`

## Build Targets

| Target | Command | Description |
|--------|---------|-------------|
| AVX2 | `./scripts/ci-build.sh avx2` | Compatible with most CPUs (2013+) |
| Vulkan | `./scripts/ci-build.sh vulkan` | GPU acceleration via Vulkan |
| AVX-512 | `./scripts/ci-build.sh avx512` | Requires AVX-512 host (build elsewhere) |
| All | `./scripts/ci-build.sh` | Builds AVX2 + Vulkan |

## Automated Builds

### Option 1: Cron Job

Add to crontab to build nightly:

```bash
# Build nightly at 2 AM
0 2 * * * cd /path/to/voxtype && git pull && ./scripts/ci-build.sh >> /var/log/voxtype-build.log 2>&1
```

### Option 2: Webhook Trigger

Create a simple webhook listener using Docker:

```bash
# webhook-listener.sh
docker run -d \
  --name voxtype-webhook \
  --restart unless-stopped \
  -p 9000:9000 \
  -v /path/to/voxtype:/repo \
  -v /var/run/docker.sock:/var/run/docker.sock \
  adnanh/webhook \
  -hooks /repo/scripts/webhooks.json \
  -verbose
```

Create `scripts/webhooks.json`:
```json
[
  {
    "id": "build",
    "execute-command": "/repo/scripts/ci-build.sh",
    "command-working-directory": "/repo"
  }
]
```

Then trigger builds with:
```bash
curl -X POST http://your-truenas:9000/hooks/build
```

### Option 3: GitHub Actions Self-Hosted Runner

1. Install the GitHub Actions runner on TrueNAS SCALE
2. Add to `.github/workflows/build.yml`:

```yaml
name: Build Binaries
on:
  push:
    tags:
      - 'v*'
jobs:
  build:
    runs-on: self-hosted
    steps:
      - uses: actions/checkout@v4
      - name: Build
        run: ./scripts/ci-build.sh
      - name: Upload artifacts
        uses: actions/upload-artifact@v4
        with:
          name: binaries
          path: releases/*/voxtype-*
```

## AVX-512 Binary

The AVX-512 binary must be built on an AVX-512 capable machine (Zen 4+, some Intel).

Options:
1. Build on your main dev machine: `docker compose -f docker-compose.build.yml --profile avx512 up avx512`
2. Use GitHub Actions with a standard runner (most have AVX-512)
3. Skip if not needed (most users are fine with AVX2)

## Verification

After building, the script automatically verifies binaries:

```
=== Verifying Binaries ===
✓ voxtype-0.4.1-linux-x86_64-avx2: Clean (no AVX-512/GFNI)
✓ voxtype-0.4.1-linux-x86_64-vulkan: Clean (no AVX-512/GFNI)
```

If verification fails, the build is aborted.

## Troubleshooting

### Build hangs on Vulkan
The Kompute shader compilation can take 30+ minutes. Be patient or reduce parallelism:
```bash
export CARGO_BUILD_JOBS=1
./scripts/ci-build.sh vulkan
```

### Docker permission denied
Add your user to the docker group:
```bash
sudo usermod -aG docker $USER
```

### Out of memory
Vulkan builds use ~4GB RAM. Limit parallelism:
```bash
export CARGO_BUILD_JOBS=1
export CMAKE_BUILD_PARALLEL_LEVEL=1
```
