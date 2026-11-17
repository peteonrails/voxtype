# Invalid Model Rejection

Verify bad model names are rejected before anything records:

```bash
# record start refuses an unknown model instead of recording with the default
voxtype --model nonexistent record start; echo "exit=$?"
voxtype status

# Expected behavior:
# 1. Only this error, exit 1: "Unknown model 'nonexistent'. Run `voxtype info models` to list available models."
#    (no "using default model" warning: that fallback applies to other commands, not record)
# 2. No recording starts: status stays "idle"

# Other commands still fall back to the default model and say so:
voxtype --model nonexistent config 2>&1 | grep -i "unknown model"
# Expected: WARN "Unknown model 'nonexistent', using default model '...'"

# The setup --set command should still reject invalid models:
voxtype setup model --set nonexistent
# Expected: error about model not installed
```

