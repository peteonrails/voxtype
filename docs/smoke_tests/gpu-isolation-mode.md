# GPU Isolation Mode

Tests subprocess-based GPU memory release (for laptops with hybrid graphics):

```bash
# 1. Enable gpu_isolation in config.toml:
#    [whisper]
#    gpu_isolation = true

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Record and transcribe
voxtype record start && sleep 3 && voxtype record stop

# 4. Check logs for subprocess spawning:
journalctl --user -u voxtype --since "1 minute ago" | grep -i subprocess

# 5. Verify GPU memory is released after transcription:
#    (AMD) watch -n1 "cat /sys/class/drm/card*/device/mem_info_vram_used"
#    (NVIDIA) nvidia-smi
```

