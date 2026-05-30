# Config Validation

Verify malformed config files produce clear errors:

```bash
# Backup current config
cp ~/.config/voxtype/config.toml ~/.config/voxtype/config.toml.bak

# Test with invalid TOML syntax
echo "invalid toml [[[" >> ~/.config/voxtype/config.toml
voxtype config  # Should show parse error with line number

# Test with unknown field (should warn but continue)
echo 'unknown_field = "value"' >> ~/.config/voxtype/config.toml
voxtype config

# Restore config
mv ~/.config/voxtype/config.toml.bak ~/.config/voxtype/config.toml
```

