# Config Validation

Since 1.1.0 the config parses section by section (#646). One unreadable value
no longer costs you every setting you have: the bad section falls back to its
defaults and is named in the log, and everything else survives. Verify each
failure class behaves the way it should:

```bash
# Backup current config
cp ~/.config/voxtype/config.toml ~/.config/voxtype/config.toml.bak

# 1. Invalid TOML syntax is still fatal (the file cannot be parsed at all).
#    Expect a parse error with a line number, not a panic.
echo "invalid toml [[[" >> ~/.config/voxtype/config.toml
voxtype config

# 2. Duplicate keys are a TOML syntax error, so they are fatal too.
#    (This one bites when a tool appends a key that already exists.)
cp ~/.config/voxtype/config.toml.bak ~/.config/voxtype/config.toml
printf '\n[osd]\nstyle = "a"\nstyle = "b"\n' >> ~/.config/voxtype/config.toml
voxtype config

# 3. One bad VALUE is salvaged. The [audio] section here falls back to
#    defaults and the log names the section (never the value, since some
#    sections hold API keys). Everything else loads. Exit code is 0.
cp ~/.config/voxtype/config.toml.bak ~/.config/voxtype/config.toml
printf '\n[audio]\nsample_rate = "not-a-number"\n' >> ~/.config/voxtype/config.toml
voxtype info engines

# 4. Unknown fields are silently ignored, so an old config with removed
#    fields (or a new config on an old binary) still works.
cp ~/.config/voxtype/config.toml.bak ~/.config/voxtype/config.toml
echo 'unknown_field = "value"' >> ~/.config/voxtype/config.toml
voxtype config

# Restore config
mv ~/.config/voxtype/config.toml.bak ~/.config/voxtype/config.toml
```
