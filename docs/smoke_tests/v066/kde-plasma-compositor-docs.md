# KDE Plasma Compositor Docs (#296)

Verifies KDE Plasma keybinding docs are present.

```bash
grep -c "KWin\|KDE Plasma" README.md docs/USER_MANUAL.md docs/CONFIGURATION.md
# Expected: matches in all three files
```

