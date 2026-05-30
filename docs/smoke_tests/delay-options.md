# Delay Options

```bash
# Test type delays (edit config.toml):
#    type_delay_ms = 50       # Inter-keystroke delay
#    pre_type_delay_ms = 200  # Pre-typing delay

systemctl --user restart voxtype
voxtype record start && sleep 2 && voxtype record stop

# Check debug logs for delay application:
journalctl --user -u voxtype --since "30 seconds ago" | grep -E "delay|sleeping"
```

