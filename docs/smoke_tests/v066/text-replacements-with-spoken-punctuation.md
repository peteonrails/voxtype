# Text Replacements with Spoken Punctuation (#172)

Verifies that text replacements match spoken words before punctuation conversion.

```bash
# Unit tests (no mic needed)
cargo test replacements_match_spoken -- --nocapture
cargo test replacements_with_multiple -- --nocapture
# Expected: both tests pass

# Runtime test (requires mic and config change):
# 1. Add to config.toml:
#    [text]
#    spoken_punctuation = true
#    replacements = [
#      { from = "slash pr", to = "/pr" },
#    ]
# 2. Restart daemon, record "slash pr one two three"
# Expected: "/pr one two three" (not "/ pr one two three")
```

