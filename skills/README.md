# Agent skills for Voxtype users

These are [Agent Skills](https://docs.claude.com/en/docs/claude-code/skills): instructions a coding agent loads when a task matches them. They are for people who use Voxtype, not for working on Voxtype itself.

| Skill | What it does |
|-------|--------------|
| [`voxtype-setup`](voxtype-setup/SKILL.md) | Inspects your hardware, asks what you dictate, and configures Voxtype's engine, model, GPU acceleration, threads and memory use to suit the machine |

## Installing

Voxtype installs the skill for the coding agents it finds on your machine:

```bash
voxtype setup skill
```

That covers Claude Code (`~/.claude/skills`) and the agents that read
`~/.agents/skills`: Codex, Gemini CLI, OpenCode, Cursor, GitHub Copilot and
Goose. `voxtype setup skill --help` lists the options.

The skill is built into the voxtype binary, so the installed copy matches your
version. Run the command again after upgrading voxtype. `voxtype setup skill
--status` shows whether each copy is current.

Then ask your agent something like "set up voxtype for this machine". The
skill shows you its plan and asks before it changes your config or runs
anything with `sudo`.

To install it by hand instead, copy `skills/voxtype-setup/` into
`~/.claude/skills/` or `~/.agents/skills/`.
