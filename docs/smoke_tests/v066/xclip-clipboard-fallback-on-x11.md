# Xclip Clipboard Fallback on X11 (#256)

Verifies xclip is in the clipboard mode output chain.

```bash
# Structural verification
grep -A5 "OutputMode::Clipboard =>" src/output/mod.rs | grep -c "XclipOutput"
# Expected: 1

# Config verification
voxtype config 2>&1 | grep -A10 "Output Chain"
# Expected: shows wl-copy and xclip detection status
```

