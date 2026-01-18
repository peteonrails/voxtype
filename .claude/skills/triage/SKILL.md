---
name: triage
description: Triage a GitHub issue or PR by checking for duplicates, documentation alignment, project fit, and recommending labels.
user-invocable: true
allowed-tools:
  - Bash(gh issue view:*)
  - Bash(gh issue list:*)
  - Bash(gh pr view:*)
  - Bash(gh pr list:*)
  - Bash(gh search issues:*)
  - Bash(gh api repos/peteonrails/voxtype/discussions:read-only)
  - Read
  - Glob
  - Grep
---

# Read-Only Constraint

**IMPORTANT**: This skill performs READ-ONLY operations. Never use:
- `gh issue edit`, `gh issue close`, `gh pr merge`
- `gh api -X POST/PUT/PATCH/DELETE`
- Any command that modifies issues, PRs, or labels

Report findings to the user; let them decide what actions to take.

# Issue/PR Triage

Triage a GitHub issue or pull request by analyzing duplicates, documentation alignment, project fit, and recommending labels.

## Usage

```
/triage <issue-or-pr-number>
```

## Workflow

### 1. Fetch the Issue/PR

Use GitHub CLI to get full details:

```bash
# For issues
gh issue view <number> --json number,title,body,labels,state,author,createdAt

# For PRs
gh pr view <number> --json number,title,body,labels,state,author,createdAt,files,additions,deletions
```

Determine the type based on content: bug report, feature request, question, documentation issue, or pull request.

### 2. Duplicate and Prior Decision Search

Search for related closed issues and discussions:

```bash
# Search closed issues with relevant labels
gh issue list --state closed --label wontfix --search "<keywords>" --limit 10
gh issue list --state closed --label duplicate --search "<keywords>" --limit 10
gh issue list --state closed --label invalid --search "<keywords>" --limit 10

# General search across all issues
gh search issues "<keywords>" --repo peteonrails/voxtype --limit 15

# Search discussions for prior decisions
gh api repos/peteonrails/voxtype/discussions --jq '.[].title' 2>/dev/null || echo "No discussions API access"
```

Extract 3-5 keywords from the issue title and body for searching.

### 3. Related PR Search

Find open PRs that might address this issue:

```bash
gh pr list --state open --search "<keywords>" --limit 10
```

### 4. Documentation Check

Read relevant documentation to determine if user misunderstands setup or configuration:

- `README.md` - Installation and quick start
- `docs/TROUBLESHOOTING.md` - Common issues and solutions
- `docs/CONFIGURATION.md` - Config file options
- `docs/PARAKEET.md` - Parakeet-specific setup (experimental)
- `docs/MODEL_SELECTION_GUIDE.md` - Model and engine selection

If the reported issue is actually documented expected behavior or a setup mistake, note this in the report.

### 5. Project Fit Assessment

Read `CLAUDE.md` sections on Project Principles and Non-Goals.

**Project Principles** (all work should align with these):
1. Dead simple user experience
2. Backwards compatibility
3. Performance first
4. Excellent CLI help
5. Every option configurable everywhere
6. Documentation in the right places

**Non-Goals** (requests for these should be declined):
- Windows/macOS support (Linux-first, Wayland-native)
- GUI configuration (CLI and config file are the interface)

Also check:
- Issues/discussions closed with decisions against similar features
- CLAUDE.md roadmap for what's planned vs out of scope

### 6. PR Quality Assessment (PRs only)

For pull requests, additionally assess:

- **Linked issue**: Does it reference an issue? Is that issue valid?
- **Scope**: Does it do exactly what the linked issue asks, or does it include scope creep?
- **Code style**: Does it follow conventions in CLAUDE.md?
- **Tests**: Are there tests for new functionality?
- **Documentation**: Are user-facing changes documented?
- **Breaking changes**: Does it break backwards compatibility?

### 7. Label Recommendation

Based on analysis, recommend appropriate labels from:

| Label | When to Apply |
|-------|---------------|
| `bug` | Confirmed bug in existing functionality |
| `enhancement` | Feature request or improvement |
| `documentation` | Documentation issue or improvement |
| `question` | User asking for help, not reporting a bug |
| `duplicate` | Already reported in another issue |
| `invalid` | Not a valid issue (spam, off-topic, etc.) |
| `wontfix` | Valid but won't be addressed (non-goal, out of scope) |
| `good first issue` | Well-defined, isolated, good for newcomers |
| `help wanted` | Needs community contribution |
| `deb` | Related to Debian packaging |
| `rpm` | Related to RPM packaging |
| `aur` | Related to AUR source package |
| `aur-bin` | Related to AUR binary package |
| `nixos` | Related to NixOS packaging |
| `omarchy` | Related to Omarchy distribution |

Apply multiple labels when appropriate (e.g., `bug` + `deb` for a Debian-specific bug).

## Output Format

Generate a triage report with this structure:

```markdown
## Triage Report: #<number> "<title>"

**Type**: Bug / Enhancement / Question / PR
**Author**: @username
**Created**: YYYY-MM-DD

### Summary
Brief description of what the issue/PR is about.

### Duplicate Check
- [ ] No duplicates found
OR
- Similar: #XX "title" (closed, wontfix) - brief reason
- Related: #YY "title" (open) - how it relates

### Documentation Check
- [ ] Not a documentation/setup issue
OR
- This appears to be a setup issue. docs/TROUBLESHOOTING.md section "XYZ" addresses this.
- User may be misunderstanding [specific feature/behavior].

### Project Fit
- [ ] Aligns with project goals
OR
- Conflicts with non-goal: [specific non-goal]
- Conflicts with principle: [specific principle]

### PR Quality (if applicable)
- Linked issue: #XX or "None"
- Scope: Appropriate / Contains scope creep
- Code style: Follows conventions / Issues noted
- Tests: Present / Missing
- Breaking changes: None / Yes - [description]

### Recommended Labels
`label1`, `label2`, `label3`

### Suggested Action
- Close as duplicate of #XX
- Close as wontfix (non-goal)
- Request more information from author
- Ready for review
- Needs documentation update before merge
- Add to backlog
- etc.
```

## Commands Reference

```bash
# View issue details
gh issue view 123 --json number,title,body,labels,state,author,createdAt

# View PR with changed files
gh pr view 123 --json number,title,body,labels,state,author,files

# Search issues by keyword
gh search issues "audio pipewire" --repo peteonrails/voxtype

# List issues by label
gh issue list --label bug --state open

# List closed wontfix issues
gh issue list --state closed --label wontfix --limit 20

# Add labels (reference only - report to user, don't auto-apply)
gh issue edit 123 --add-label "bug,deb"

# Close with comment (reference only)
gh issue close 123 --comment "Closing as duplicate of #XX"
```
