---
name: update-docs
description: Update voxtype documentation for releases and features. Use when adding features, fixing bugs, or preparing releases. Covers user manual, troubleshooting, website, release notes, and contributor credits.
user-invocable: true
allowed-tools:
  - Read
  - Edit
  - Write
  - Glob
  - Grep
  - Bash
  - Skill
---

# Update Documentation

Guide for updating voxtype documentation across all locations.

## Documentation Locations

| Document | Location | When to Update |
|----------|----------|----------------|
| User Manual | `docs/USER_MANUAL.md` | New features, usage changes |
| Configuration Guide | `docs/CONFIGURATION.md` | New config options |
| Troubleshooting | `docs/TROUBLESHOOTING.md` | New error conditions, fixes |
| FAQ | `docs/FAQ.md` | Common questions |
| README | `README.md` | Major features, installation changes |
| User setup skill | `skills/voxtype-setup/SKILL.md` | Every release: new or changed commands, config keys, engines, models, output options |
| Website News | `website/news/index.html` | Every release |
| GitHub Release | GitHub Releases | Every release |
| Obsidian Vault | `~/Documents/markdown-notes/Voxtype/` | Session notes, decisions |

## Writing Style

**Avoid AI writing patterns:**
- No em-dashes (—) - use colons or separate sentences
- No "delve", "leverage", "utilize", "streamline", "robust", "seamless"
- No excessive hedging ("It's worth noting...", "Interestingly...")
- No punchy one-liners ("And that's the point.", "Simple as that.")
- No sentence fragments for effect ("The result? Faster builds.")
- Write plainly and directly

## User Manual Updates

Location: `docs/USER_MANUAL.md`

When adding a feature:
1. Add to table of contents if significant
2. Create new section with clear heading
3. Include usage examples with code blocks
4. Explain the "why" not just the "how"

## Configuration Guide Updates

Location: `docs/CONFIGURATION.md`

For new config options:
1. Add to the relevant section
2. Show the TOML syntax
3. Document the default value
4. Explain what it does and when to use it

```markdown
### new_option

Enable the new feature.

```toml
[section]
new_option = true  # default: false
```

When enabled, this does X which is useful for Y.
```

## Troubleshooting Updates

Location: `docs/TROUBLESHOOTING.md`

Format for new issues:

```markdown
### Error: The specific error message

**Cause:** Why this happens

**Solution:**
1. Step one
2. Step two

**Example:**
```bash
command to fix
```
```

## Website News Articles

Location: `website/news/index.html`

Every GitHub release needs a matching news article. Follow v0.4.10/v0.4.11 as examples.

Structure:
```html
<article class="news-article" id="v0415">
    <div class="article-meta">
        <time datetime="2026-01-20">January 20, 2026</time>
        <span class="article-tag">Release</span>
    </div>
    <h2>v0.4.15: Feature Summary</h2>
    <div class="article-body">
        <p>Intro paragraph...</p>

        <h3>Feature Name</h3>
        <p>Description.</p>
        <p><strong>Why use it:</strong> User benefit.</p>

        <div class="code-block">
            <div class="code-header"><span>config.toml</span></div>
            <pre><code>[section]
option = "value"</code></pre>
        </div>
    </div>
</article>
```

Add new articles at the TOP of the articles list.

## GitHub Release Notes

Format:
- Title: `v0.4.15: Feature Summary`
- Brief intro paragraph
- `###` sections for each feature
- `**Why use it:**` callouts
- Bug fixes as bullet list
- Downloads table with checksums

## Obsidian Vault Notes

Use the `/obsidian` skill to save session context.

Location: `~/Documents/markdown-notes/Voxtype/`

Include:
- Summary of work completed
- Key decisions and rationale
- Links to PRs/issues
- Next steps

## Crediting Contributors

### In Commits

Add co-authors to commit messages:
```
Co-authored-by: Name <email@example.com>
```

### In Cargo.toml

Add to the authors array:
```toml
authors = [
    "Peter Jackson <pete@peteonrails.com>",
    "Contributor Name <contributor@example.com>",
]
```

### In README

Add to Contributors section with GitHub profile link.

### On Website

Add to the website's contributors or about page if significant contribution.

## Release Documentation Checklist

- [ ] Update `docs/USER_MANUAL.md` for new features
- [ ] Update `docs/CONFIGURATION.md` for new options
- [ ] Update `docs/TROUBLESHOOTING.md` for new error conditions
- [ ] Add news article to `website/news/index.html`
- [ ] Create GitHub release with notes
- [ ] Update `packaging/arch-bin/voxtype-bin.install` post_upgrade message
- [ ] Credit contributors in Cargo.toml and README
- [ ] Check `skills/voxtype-setup/SKILL.md`: every command and config key it names still exists (`voxtype --help`, `voxtype config schema`), its recommendations cover what shipped, and anything it calls planned (built-in history, #209) is updated once released
- [ ] Save session notes to Obsidian vault

## CLI Help Text

When adding new CLI options, update the clap attributes in `src/cli.rs`:

```rust
/// Clear description of what this option does
#[arg(long, short = 'x')]
pub new_option: bool,
```

The `--help` output IS documentation. Make it clear and complete.
