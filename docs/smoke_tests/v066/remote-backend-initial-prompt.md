# Remote Backend initial_prompt (#278)

Verifies that initial_prompt is forwarded to remote transcription endpoints.

```bash
# Unit tests (no remote server needed)
cargo test multipart_body_includes_prompt -- --nocapture
cargo test multipart_body_excludes -- --nocapture
# Expected: all 3 tests pass (includes, excludes_empty, excludes_when_none)
```

