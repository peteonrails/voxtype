# dotool Keyboard Layout

Tests keyboard layout support for non-US keyboards:

```bash
# 1. Add keyboard layout to config.toml:
#    [output]
#    dotool_xkb_layout = "de"        # German layout
#    dotool_xkb_variant = "nodeadkeys"  # Optional variant

# 2. Hide wtype to force dotool
sudo mv /usr/bin/wtype /usr/bin/wtype.bak

# 3. Restart daemon and test
systemctl --user restart voxtype
voxtype record start && sleep 2 && voxtype record stop

# 4. Verify layout is applied (check dotool receives DOTOOL_XKB_LAYOUT env var):
journalctl --user -u voxtype --since "30 seconds ago" | grep -i "keyboard layout"

# 5. Restore wtype
sudo mv /usr/bin/wtype.bak /usr/bin/wtype
```

