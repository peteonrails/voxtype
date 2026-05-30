# Single Instance Enforcement

Verify only one daemon can run at a time:

```bash
# With daemon already running via systemd, try starting another:
voxtype daemon
# Should fail with error about existing instance / PID lock

# Check PID file:
cat ~/.local/share/voxtype/voxtype.pid
ps aux | grep voxtype
```

