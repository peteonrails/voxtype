# Eitype in Paste Mode (#259)

Verifies eitype is in the paste mode Ctrl+V simulation chain.

```bash
# Structural verification
grep -c "simulate_paste_eitype\|is_eitype_available" src/output/paste.rs
# Expected: 6+ references

# Runtime test (requires eitype installed):
# 1. Set mode = "paste" in config.toml
# 2. Hide wtype: sudo mv /usr/bin/wtype /usr/bin/wtype.bak
# 3. Record and transcribe
# 4. Check logs: journalctl --user -u voxtype --since "30 seconds ago" | grep -i eitype
# 5. Restore: sudo mv /usr/bin/wtype.bak /usr/bin/wtype
```

