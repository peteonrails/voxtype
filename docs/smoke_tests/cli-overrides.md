# CLI Overrides

```bash
# Output mode override (use --clipboard, --type, or --paste)
voxtype record start --clipboard
sleep 2
voxtype record stop
# Verify clipboard has text: wl-paste

# Model override (requires model to be downloaded)
# Note: --model flag is on the main command, not record subcommand
voxtype --model base.en record start
sleep 2
voxtype record stop
```

