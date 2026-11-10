#!/usr/bin/env python3
"""Real CLI and PTY save paths for the shared Handy GGUF engine."""
import argparse, fcntl, json, os, pathlib, pty, select, signal, struct, subprocess, tempfile, termios, time, tomllib

p = argparse.ArgumentParser()
p.add_argument("--bin", required=True, type=pathlib.Path)
p.add_argument("--models", required=True, type=pathlib.Path)
p.add_argument("--out", required=True, type=pathlib.Path)
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=True)
binary = a.bin.resolve()
results = []

with tempfile.TemporaryDirectory(dir=a.out) as temporary:
    root = pathlib.Path(temporary)
    config = root / "config.toml"
    data = root / "data"
    store = data / "voxtype/models"
    store.mkdir(parents=True)
    for name in ["Qwen3-ASR-0.6B-Q8_0.gguf", "Qwen3-ASR-1.7B-Q8_0.gguf"]:
        (store / name).symlink_to((a.models / name).resolve())
    worker = root / "worker"
    worker.write_text("#!/bin/sh\nexit 99\n")
    worker.chmod(0o755)
    original = '# exact legacy config\nengine = "whisper"\n[whisper]\nmodel = "base.en" # keep\n'
    env = {k: v for k, v in os.environ.items() if not k.startswith("VOXTYPE_")}
    env.update(HOME=str(root), XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(data), XDG_CACHE_HOME=str(root / "cache"), TERM="xterm-256color", VOXTYPE_GGUF_WORKER=str(worker))

    def run(name, args, success=True, reset=False):
        if reset:
            config.write_text(original)
        before = config.read_bytes()
        result = subprocess.run([str(binary), "--config", str(config), *args], env=env, capture_output=True, text=True, timeout=40)
        case = a.out / name
        case.mkdir(exist_ok=True)
        (case / "stdout").write_text(result.stdout)
        (case / "stderr").write_text(result.stderr)
        (case / "exit").write_text(f"{result.returncode}\n")
        (case / "before.toml").write_bytes(before)
        (case / "after.toml").write_bytes(config.read_bytes())
        assert (result.returncode == 0) == success, (name, result.stdout, result.stderr)
        if not success:
            assert config.read_bytes() == before
        results.append({"case": name, "exit": result.returncode, "passed": True})
        return result

    config.write_text(original)
    run("engine-picker-save", ["config", "set", "engine", "gguf"])
    saved = tomllib.loads(config.read_text())
    assert saved["engine"] == "gguf"
    assert saved["gguf"]["model"] == "Qwen3-ASR-0.6B-Q8_0.gguf"
    assert saved["gguf"]["architecture"] == "qwen3_asr"
    assert saved["gguf"]["language"] == "auto"
    run("model-picker-save", ["config", "set", "gguf.model", "Qwen3-ASR-1.7B-Q8_0.gguf"])
    saved = tomllib.loads(config.read_text())
    assert saved["gguf"]["model"] == "Qwen3-ASR-1.7B-Q8_0.gguf"
    assert saved["gguf"]["architecture"] == "qwen3_asr"
    run("reject-streaming", ["config", "set", "gguf.streaming", "true"], success=False)
    catalog = run("catalog", ["info", "models", "--engine", "gguf", "--json"])
    assert len(json.loads(catalog.stdout)["engines"]["gguf"]["models"]) == 5
    config.write_text(original)
    run("setup-download-activate", ["setup", "--quiet", "--no-post-install", "--download", "--model", "Qwen3-ASR-0.6B-Q8_0.gguf", "--activate"])
    saved = tomllib.loads(config.read_text())
    assert saved["engine"] == "gguf" and saved["gguf"]["architecture"] == "qwen3_asr"

    config.write_text(original)
    pid, fd = pty.fork()
    if pid == 0:
        os.execve(str(binary), [str(binary), "--config", str(config), "configure"], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 42, 180, 0, 0))
    raw = bytearray()
    def drain(seconds=0.7):
        until = time.monotonic() + seconds
        while time.monotonic() < until:
            if select.select([fd], [], [], 0.05)[0]:
                chunk = os.read(fd, 65536)
                if not chunk:
                    break
                raw.extend(chunk)
    try:
        drain(2)
        os.write(fd, b"j\rllls")
        drain(2)
        saved = tomllib.loads(config.read_text())
        assert saved["engine"] == "gguf", saved
        assert saved["gguf"]["model"] == "Qwen3-ASR-0.6B-Q8_0.gguf", saved
        results.append({"case": "tui-engine-model-save", "passed": True})
    finally:
        (a.out / "tui.raw").write_bytes(raw)
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
        os.close(fd)

(a.out / "results.json").write_text(json.dumps(results, indent=2) + "\n")
print(json.dumps(results, indent=2))
