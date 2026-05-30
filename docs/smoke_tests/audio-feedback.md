# Audio Feedback

```bash
# Enable audio feedback in config.toml:
#    [audio.feedback]
#    enabled = true
#    theme = "default"
#    volume = 0.5

systemctl --user restart voxtype
voxtype record start  # Should hear start beep
sleep 2
voxtype record stop   # Should hear stop beep
```

