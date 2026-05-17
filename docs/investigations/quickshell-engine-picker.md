# Investigation: Quickshell engine picker — missing CLI/IPC entry point

**Status:** Blocked. Engine switching has no programmatic CLI/IPC entry point
that the Quickshell engine picker can invoke. Recommend adding a small
`voxtype config set` CLI surface before shipping the widget.

**Wave:** v0.7.3 Wave 2 (Quickshell composition pass)
**Related:** `project_quickshell_v073.md` item 2

## Goal

Wave 2 of the Quickshell v0.7.3 work ships an engine picker — a QML panel
that lists installed transcription engines (whisper, parakeet, moonshine,
sensevoice, paraformer, dolphin, omnilingual, cohere), marks the active
one, and lets the user click another engine to switch.

The QML side is straightforward (`PanelWindow` + `ListView` + a `Process`
to invoke whatever switches the engine). What's missing is the thing the
`Process` is supposed to invoke.

## What exists today

### 1. CLI flags (per-invocation only)

`src/cli.rs` exposes a top-level `--engine` flag and a per-subcommand
`--engine` flag on `voxtype transcribe`:

```rust
/// Override transcription engine
#[arg(long, value_name = "ENGINE")]
pub engine: Option<String>,
```

This is a process-scoped override. It does NOT persist to the config
file. Useful for one-off transcriptions, useless for "switch the daemon
to a new engine."

### 2. Environment variable (process-scoped)

`VOXTYPE_ENGINE` is read in `src/config.rs:2429`:

```rust
if let Ok(engine) = std::env::var("VOXTYPE_ENGINE") {
    match engine.to_lowercase().as_str() {
        "whisper" => config.engine = TranscriptionEngine::Whisper,
        "parakeet" => config.engine = TranscriptionEngine::Parakeet,
        // ...
    }
}
```

Same problem: this is read at daemon startup and doesn't survive across
restarts unless the user (or the systemd unit) sets the env var in their
shell. The systemd user unit doesn't currently honor a per-user env
override file for engine selection.

### 3. Config file (persistent, but no CLI to write it)

The canonical persistence path is `engine = "..."` at the root of
`~/.config/voxtype/config.toml`. The daemon reads this at startup. The
TUI's `src/tui/engine.rs` writes it via `ConfigEditor::set_string("",
"engine", &self.engine)` followed by `ConfigEditor::save()` (atomic
temp-file + rename, with validation).

`ConfigEditor` is a Rust struct, not a CLI surface. The only way to
trigger it from outside the voxtype process is to launch the TUI
(`voxtype configure`), navigate to the engine pane, edit, save, and
exit — which is exactly the manual workflow the picker is meant to
replace.

### 4. Daemon has no config hot-reload

`src/daemon.rs` handles `SIGUSR1` (start recording), `SIGUSR2` (stop),
and `SIGTERM`. No `SIGHUP`, no config-reload IPC, no Unix socket for
configuration commands. After the config file is rewritten, the daemon
must be restarted to pick up the new engine:

```bash
systemctl --user restart voxtype
```

(The CLAUDE.md "Development Notes" section already documents this:
restarting via systemctl also takes Waybar status followers with it,
which need a `pkill -SIGUSR2 waybar` to reconnect.)

## What's missing

A CLI command that:

1. Validates the requested engine against the compiled-in features (so
   we reject `cohere` on a Whisper-only build with a helpful error
   rather than silently writing a value the daemon can't load).
2. Writes `engine = "<value>"` to the config file atomically (the
   `ConfigEditor` machinery already exists in `src/tui/config_editor.rs`
   — it just needs a non-TUI entry point).
3. Optionally restarts the user's voxtype unit, or prints a one-line
   reminder to do so.

A reasonable shape:

```bash
voxtype config set engine parakeet
# writes config, prints:
#   Engine set to parakeet. Restart daemon to apply:
#     systemctl --user restart voxtype
```

With a `--restart` flag that invokes `systemctl --user restart
voxtype.service` (and a `--no-restart` default so we don't surprise
users on hosts where voxtype isn't running under systemd).

## Why not just shell out from QML

A previous iteration of this widget could shell out from the QML
`Process` element to `sed` or `python3 -c 'import tomllib; ...'` to
patch `engine = "..."` in the config file. That path is explicitly
rejected for several reasons:

1. **No validation.** A typo writes `engine = "moonshien"` and the
   daemon silently falls back to default on next start.
2. **No feature gating.** Writing `engine = "cohere"` on a build
   without the `cohere` feature lands the user in an unbootable
   daemon. The TUI's engine pane already does this check via
   `compiled_features`; a one-shot CLI should do the same.
3. **TOML round-trip risk.** The existing config has comments,
   ordering, and nested tables that a naive `sed` will mangle.
   `ConfigEditor` already handles this correctly with `toml_edit`.
4. **Distros and audit.** Having voxtype's own widget edit voxtype's
   own config via a third-party Python interpreter is the kind of
   thing that fails on minimal Hyprland setups where `python3` isn't
   guaranteed to be installed.

The half-baked widget Pete warned against in the Wave 2 brief is
exactly this — shell out to an external tool to patch our own config.

## Proposed scope

Two PRs:

1. **`voxtype config set` CLI** (small, ships independently). Add a
   `Set { key, value }` variant under the existing `Config` subcommand
   or as a new `ConfigAction` enum. Initially supports `engine` only;
   the structure leaves room for `voxtype config set whisper.model
   large-v3-turbo` later. Reuses `ConfigEditor` from
   `src/tui/config_editor.rs` (currently `pub(crate)` — promote to
   `pub` or expose a `set_engine()` helper that wraps it).
2. **Quickshell engine picker** (this work). Once #1 lands, the QML
   widget invokes `voxtype config set engine <name> --restart` via
   `Process` and watches `voxtype info variants --json` (or a new
   `voxtype info engine --json`) to populate the list.

The dependency is clean: the picker is purely a QML/Process consumer,
and `voxtype config set` is useful on its own (scripted setup,
Ansible/Nix provisioning, post-install hooks).

## Alternatives considered

- **Daemon SIGHUP for hot reload.** Tempting but invasive. Engine
  switches require model swaps, GPU memory release, and recreation of
  the `Transcriber` instance — non-trivial in the current daemon
  topology where `transcriber_preloaded` is bound in `run_loop()`. Out
  of scope for v0.7.3.
- **Unix socket IPC for config commands.** Same blocker as SIGHUP:
  the daemon's `tokio::select!` loop doesn't currently have a
  config-mutation surface, and we'd need to design one that doesn't
  fight the engine reload work above. Pete's brief explicitly says
  "Don't add a new daemon IPC channel — use existing CLI / config
  mechanisms. (We can add a proper IPC in v0.7.4 if needed.)"
- **Run the TUI in `--engine X` mode and auto-save.** Cute but
  brittle; the TUI is interactive by design and adding a
  scripting-only mode is more code than just promoting
  `ConfigEditor::set_engine` to a CLI.

## Recommendation

Land `voxtype config set engine <name>` first (separate PR, ~50 lines
of Rust + a smoke test). Then ship the Quickshell engine picker as
this PR's follow-up. The widget itself is trivial once the underlying
CLI exists.

The maintainer/Pete should decide whether to:

a) Land `voxtype config set` in v0.7.3 alongside the rest of Wave 2,
b) Defer the engine picker to v0.7.4 and ship Wave 2 without it, or
c) Accept a placeholder picker that surfaces "engine switching from
   QML is not yet supported — open `voxtype configure`" as its action,
   buying time for the CLI work in v0.7.4.

This investigation is the artifact for option (c)-as-default until
the call is made.
