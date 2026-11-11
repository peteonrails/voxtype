# Agent skills for Voxtype users

These are [Agent Skills](https://docs.claude.com/en/docs/claude-code/skills): instructions a coding agent loads when a task matches them. They are for people who use Voxtype, not for working on Voxtype itself.

| Skill | What it does |
|-------|--------------|
| [`voxtype-setup`](voxtype-setup/SKILL.md) | Inspects your hardware, asks what you dictate, and configures Voxtype's engine, model, GPU acceleration, threads and memory use to suit the machine |

## Installing

For Claude Code, copy the skill's directory into your personal skills folder:

```bash
mkdir -p ~/.claude/skills
cp -r skills/voxtype-setup ~/.claude/skills/
```

Then ask your agent something like "set up voxtype for this machine". Other agents that read `SKILL.md` files load them from their own skills directory, so check your agent's documentation for the path.

The skill shows you its plan and asks before it changes your config or runs anything with `sudo`.
