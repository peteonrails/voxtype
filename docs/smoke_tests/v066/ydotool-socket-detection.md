# Ydotool Socket Detection (#306)

Verifies ydotool socket is found at non-standard paths (Fedora).

```bash
# Unit tests
cargo test find_ydotool_socket -- --nocapture
# Expected: 2 tests pass (env override and returns_none)

# Structural verification
grep -c "find_ydotool_socket" src/output/ydotool.rs src/output/paste.rs
# Expected: references in both files
```

