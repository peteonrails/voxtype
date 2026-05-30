# Setup retry-hint echoes invoked subcommand (#449)

Previously, running `voxtype setup gpu --enable` without root suggested
retrying as `sudo voxtype setup onnx --enable`, which is a different
command. Now the retry hint matches the subcommand the user invoked, with
`switch_to` deriving the hint from the variant family for the TUI path.

## Structural verification

```bash
grep -c "Try: sudo voxtype " src/setup/binary.rs
# Expected: 3 (the three error paths)

grep -c "retry_hint" src/setup/binary.rs src/setup/gpu.rs
# Expected: 5+ (param threaded through callers)
```

Confirm the literal `setup onnx --enable` hint is no longer hard-coded in
`binary.rs` (it's still in the wrapper script's audit comment, which is
intentional: wrappers are only generated for ONNX variants).

```bash
grep -c 'Try: sudo voxtype setup onnx' src/setup/binary.rs
# Expected: 0
```

## Runtime tests

These require not having sudo cached and ideally running as a user
without write access to `/usr/bin/voxtype` (e.g., a stock package install).

### Path 1: `setup gpu`

```bash
voxtype setup gpu --enable 2>&1 | grep "Try:"
# Expected: Try: sudo voxtype setup gpu --enable
```

### Path 2: `setup onnx`

```bash
voxtype setup onnx --enable 2>&1 | grep "Try:"
# Expected: Try: sudo voxtype setup onnx --enable
```

### Path 3: TUI variant picker

Without sudo cached, pick a Whisper variant in the TUI's General section
and observe the on-screen error after pkexec cancellation. The retry
hint should be `Try: sudo voxtype setup gpu --enable`.

Then pick an ONNX variant under the same conditions. The retry hint
should be `Try: sudo voxtype setup onnx --enable`.
