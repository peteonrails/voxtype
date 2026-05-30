# Post-Processing Command

Tests LLM cleanup if configured:

```bash
# 1. Configure post-processing in config.toml:
#    [output]
#    post_process_command = "your-llm-cleanup-script"

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Record and transcribe
voxtype record start && sleep 3 && voxtype record stop

# 4. Check logs for post-processing:
journalctl --user -u voxtype --since "1 minute ago" | grep -i "post.process"
```

