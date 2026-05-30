# Engine Switching via Model Selection

Test that selecting a model from a different engine updates config correctly:

```bash
# Start with Whisper engine configured
grep engine ~/.config/voxtype/config.toml  # Should show engine = "whisper" or be absent

# Select a Parakeet model (requires --features parakeet build)
voxtype setup model  # Choose a parakeet-tdt model
grep engine ~/.config/voxtype/config.toml  # Should show engine = "parakeet"
grep -A2 "\[parakeet\]" ~/.config/voxtype/config.toml  # Should show model name

# Select a Whisper model
voxtype setup model  # Choose a Whisper model (e.g., base.en)
grep engine ~/.config/voxtype/config.toml  # Should show engine = "whisper"

# Verify star indicator shows current model
voxtype setup model  # Current model should have * prefix
```

