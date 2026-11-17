# Single Instance Enforcement

Verify only one daemon can run at a time:

```bash
# With daemon already running via systemd, try starting another:
voxtype daemon
# Should fail with error about existing instance / PID lock

# Check PID file:
cat "$XDG_RUNTIME_DIR/voxtype/pid"
ps aux | grep voxtype
```

