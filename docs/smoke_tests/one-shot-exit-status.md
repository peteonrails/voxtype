# One-shot exit status (ONNX/CUDA abort at teardown)

Test that a one-shot command exits with a status code and never a signal, on
every build, and that the CUDA variant no longer aborts after a successful
transcription.

## Why

`voxtype transcribe <file>` on the ONNX variants printed its result and then
aborted during static destructor teardown: `corrupted double-linked list`,
exit 134, one core dump per run ([#772](https://github.com/peteonrails/voxtype/issues/772)).
The one-shot CLI exit path now leaves the process without running static
destructors, the same way the daemon does (commit `525a0a6d`).

## Automated

```bash
cargo test --test one_shot_exit
```

Expected: one test passes. It pins the exit contract (status code, no signal,
error reported) on the default build. It cannot reproduce the abort itself,
which needs the CUDA build, the ONNX Runtime libraries, and a model.

## Manual, packaged CUDA variant

The CUDA abort is environment-bound, so check it on a host with the CUDA 13
build installed. Disable core dumps first: the abort writes ~360 MB per run.

```bash
# Reproduce the old behaviour on the release binary
ulimit -c 0
/usr/lib/voxtype/cuda-13/voxtype-onnx-cuda-13 transcribe /tmp/sil16k.wav; echo "exit=$?"

# Check the fixed binary. Stage it with its companion libs and run it ten times
mkdir -p /tmp/voxtype-fix-test && cd /tmp/voxtype-fix-test
# copy: voxtype-onnx-cuda-13, libonnxruntime.so.1.24.4, libonnxruntime.so,
#       libonnxruntime_providers_cuda.so, libonnxruntime_providers_shared.so
for i in $(seq 1 10); do
  ./voxtype-onnx-cuda-13 transcribe /tmp/sil16k.wav >/dev/null 2>&1
  echo "rc=$?"
done
```

Expected:

- Before the fix: `exit=134` on every run, `corrupted double-linked list` on
  stderr, and a new `coredumpctl list` entry per run.
- After the fix: ten `rc=0`, no `corrupted double-linked list`, no new
  `coredumpctl list` entries.
- Error paths still exit non-zero and still print `Error: ...`: a missing model
  or a zero-sample WAV returns 1, not 134.
