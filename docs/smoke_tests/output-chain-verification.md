# Output Chain Verification

Verify the complete fallback chain works:

```bash
# Check which output methods are available:
voxtype config | grep -A10 "Output Chain"

# Expected output shows installed status for each method:
#   wtype:    installed
#   dotool:   installed (if available)
#   ydotool:  installed, daemon running
#   wl-copy:  installed
```

