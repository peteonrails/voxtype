# Remote Transcription

The setting is `whisper.mode`. It was called `backend` before, and a config
file that still says `backend = "remote"` keeps working: the loader reads it
as a deprecated alias and logs a deprecation warning. `mode` wins when a file
has both.
`voxtype config set` only knows `whisper.mode`; `config set whisper.backend`
answers `unknown config key`.

```bash
# 1. Select remote transcription. With the CLI:
voxtype config set whisper.mode remote
voxtype config set whisper.remote_endpoint http://your-server:8080
#    or by hand in config.toml:
#    [whisper]
#    mode = "remote"
#    remote_endpoint = "http://your-server:8080"
#    It only applies when the engine is Whisper:
voxtype config get engine          # whisper
voxtype config get whisper.mode    # remote

# 2. Restart and test
systemctl --user restart voxtype
voxtype record start && sleep 3 && voxtype record stop

# 3. Check logs for remote transcription ("Using remote whisper transcription mode"):
journalctl --user -u voxtype --since "1 minute ago" | grep -i remote
# If that prints nothing: journalctl _SYSTEMD_USER_UNIT=voxtype.service --since "1 minute ago" | grep -i remote

# 4. The deprecated spelling. Replace `mode = "remote"` with `backend = "remote"`
#    in config.toml by hand, then:
voxtype config get whisper.mode    # still remote
systemctl --user restart voxtype
voxtype record start && sleep 3 && voxtype record stop
journalctl --user -u voxtype --since "1 minute ago" | grep -E "DEPRECATED|remote whisper"
# Expected: "DEPRECATED: [whisper] backend is deprecated, use 'mode' instead"
# and "Using remote whisper transcription mode".
```
