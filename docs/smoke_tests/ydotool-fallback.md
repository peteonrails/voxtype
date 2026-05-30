# ydotool Fallback

Tests the ydotool output driver (requires ydotoold daemon):

```bash
# Requires: ydotool installed, ydotoold running

# 1. Temporarily hide wtype and dotool to force ydotool fallback
sudo mv /usr/bin/wtype /usr/bin/wtype.bak
sudo mv /usr/bin/dotool /usr/bin/dotool.bak

# 2. Record and transcribe
voxtype record start && sleep 2 && voxtype record stop

# 3. Check logs for ydotool usage:
journalctl --user -u voxtype --since "30 seconds ago" | grep -E "ydotool|Text output"
# Expected: "dotool not available, trying next" then "Text output via ydotool"

# 4. Restore wtype and dotool
sudo mv /usr/bin/wtype.bak /usr/bin/wtype
sudo mv /usr/bin/dotool.bak /usr/bin/dotool
```

