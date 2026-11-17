# Config Validation

Since 1.1.0 the config parses section by section (#646). One unreadable value
no longer costs you every setting you have: the bad section falls back to its
defaults and is named in the log, and everything else survives. Verify each
failure class behaves the way it should.

Work on a copy through `--config`, so the running daemon never sees a broken
file. The steps edit in place rather than appending sections, because a real
config already has `[audio]`, `[osd]` and friends, and appending a second
copy of a section tests duplicate-key handling instead of what the step says.

```bash
T=$(mktemp --suffix=.toml)

# 1. Invalid TOML syntax is still fatal (the file cannot be parsed at all).
#    Expect a parse error with a line number, not a panic.
cp ~/.config/voxtype/config.toml "$T"
echo "invalid toml [[[" >> "$T"
voxtype --config "$T" config

# 2. Duplicate keys are a TOML syntax error, so they are fatal too.
#    (This one bites when a tool appends a key that already exists.)
cp ~/.config/voxtype/config.toml "$T"
printf '\n[smoke_dup]\nkey = "a"\nkey = "b"\n' >> "$T"
voxtype --config "$T" config

# 3. One bad VALUE is salvaged. The [audio] section here falls back to
#    defaults and the log names the section (never the value, since some
#    sections hold API keys). Everything else loads. Exit code is 0.
cp ~/.config/voxtype/config.toml "$T"
sed -i '/^\[audio\]/,/^\[/ s/^sample_rate = .*/sample_rate = "not-a-number"/' "$T"
grep -q 'sample_rate = "not-a-number"' "$T" || printf '\n[audio]\nsample_rate = "not-a-number"\n' >> "$T"
voxtype --config "$T" info engines

# 4. Unknown fields are silently ignored, so an old config with removed
#    fields (or a new config on an old binary) still works.
cp ~/.config/voxtype/config.toml "$T"
sed -i '1i unknown_field = "value"' "$T"
voxtype --config "$T" config

rm "$T"
```
