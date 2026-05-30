# Post-Process trim and fallback_on_empty (#270)

Verifies the post-process trim / fallback_on_empty config options end-to-end.

## Unit-level (fast)

```bash
# Behavior covered by tests in src/output/post_process.rs:
cargo test --lib output::post_process
# Expected: 21 passed (covers all four trim×fallback combinations
# plus whitespace-only output, multiline, unicode, timeout, etc.)
```

## End-to-end · trim = true (default)

```bash
# 1. Set up a post-process command that emits trailing whitespace.
#    Backup the existing config first.
cp ~/.config/voxtype/config.toml ~/.config/voxtype/config.toml.bak

cat >> ~/.config/voxtype/config.toml <<'EOF'

[post_process]
command = "sed 's/$/   /'"
trim = true
fallback_on_empty = true
EOF

systemctl --user restart voxtype

# 2. Switch output mode to file so the result is observable.
voxtype record start --file=/tmp/voxtype-trim.txt
sleep 2 && say-something-out-loud
voxtype record stop --file=/tmp/voxtype-trim.txt

# 3. Verify trailing whitespace was trimmed.
xxd /tmp/voxtype-trim.txt | tail -1
# Expected: line ends with the last spoken word's bytes, no
# trailing 0x20 0x20 0x20 (the spaces sed appended).

# 4. Restore config.
cp ~/.config/voxtype/config.toml.bak ~/.config/voxtype/config.toml
systemctl --user restart voxtype
```

## End-to-end · fallback_on_empty = true

```bash
# 1. Configure a post-process command that always returns empty.
cat >> ~/.config/voxtype/config.toml <<'EOF'

[post_process]
command = "true"   # exit 0, emit nothing
trim = true
fallback_on_empty = true
EOF

systemctl --user restart voxtype

# 2. Record and stop.
voxtype record start --file=/tmp/voxtype-fallback.txt
sleep 2 && say-something-out-loud
voxtype record stop --file=/tmp/voxtype-fallback.txt

# 3. The transcript should still appear — fallback kept the original
#    text instead of the empty post-process output.
cat /tmp/voxtype-fallback.txt
# Expected: non-empty file containing the spoken words.
```

## End-to-end · fallback_on_empty = false

```bash
# 1. Same command, but flip fallback off.
cat >> ~/.config/voxtype/config.toml <<'EOF'

[post_process]
command = "true"
trim = true
fallback_on_empty = false
EOF

systemctl --user restart voxtype

# 2. Record and stop.
voxtype record start --file=/tmp/voxtype-no-fallback.txt
sleep 2 && say-something-out-loud
voxtype record stop --file=/tmp/voxtype-no-fallback.txt

# 3. The transcript should be empty — fallback disabled, post-process
#    returned nothing, no fallback to original.
test ! -s /tmp/voxtype-no-fallback.txt && echo "PASS: empty output"
# Expected: PASS
```

## Structural verification

```bash
grep -c "trim\|fallback_on_empty" src/output/post_process.rs
# Expected: 10+ references
```

