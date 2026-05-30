# dotool Fallback

Tests the dotool output driver (supports keyboard layouts for non-US keyboards):

```bash
# Requires: dotool installed, user in 'input' group

# 1. Temporarily hide wtype to force dotool fallback
sudo mv /usr/bin/wtype /usr/bin/wtype.bak

# 2. Record and transcribe
voxtype record start && sleep 2 && voxtype record stop

# 3. Check logs for dotool usage:
journalctl --user -u voxtype --since "30 seconds ago" | grep -E "dotool|Text output"
# Expected: "wtype not available, trying next" then "Text typed via dotool"

# 4. Restore wtype
sudo mv /usr/bin/wtype.bak /usr/bin/wtype
```

