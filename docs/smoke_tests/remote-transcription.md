# Remote Transcription

```bash
# 1. Configure remote backend in config.toml:
#    [whisper]
#    backend = "remote"
#    remote_endpoint = "http://your-server:8080"

# 2. Restart and test
systemctl --user restart voxtype
voxtype record start && sleep 3 && voxtype record stop

# 3. Check logs for remote transcription:
journalctl --user -u voxtype --since "1 minute ago" | grep -i remote
```

