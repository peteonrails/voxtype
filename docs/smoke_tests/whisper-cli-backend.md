# Whisper CLI Backend (v0.5.1)

Tests the whisper-cli subprocess backend for glibc 2.42+ compatibility:

```bash
# Requires: whisper-cli installed (from whisper.cpp project)
which whisper-cli || echo "whisper-cli not installed - skip this test"

# 1. Configure CLI backend in config.toml:
#    [whisper]
#    backend = "cli"
#    # Optionally specify path:
#    # cli_path = "/usr/local/bin/whisper-cli"

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Record and transcribe
voxtype record start && sleep 3 && voxtype record stop

# 4. Check logs for CLI backend usage:
journalctl --user -u voxtype --since "30 seconds ago" | grep -i "cli"
# Expected: "Using whisper-cli subprocess backend"

# 5. Restore local backend:
#    [whisper]
#    backend = "local"
```

