# Dictation history

## Safety and evidence

Run automation only in a disposable environment with a synthetic HOME, explicit
config, private XDG data/runtime directories, no production D-Bus/compositor
sockets, no microphone/input devices, and capture-only output fixtures. Use a
private PTY for the terminal viewer and a private headless display/D-Bus for
GTK/QML. Do not use the user's clipboard, current focused app, existing history
or installed daemon as a fixture.

Pin the complete source archive/tree, toolchain image, actual producer exits,
executable and external QML/package bytes. Retain original failures. A corrected
capture harness is a separate result, not a retroactive pass. Decode PTY cursor
positioning before checking layout; token presence in raw output is not layout
acceptance. Never add repeated runs to the count of distinct regression cases.

This catalogue describes acceptance checks, not a record that they passed.
Mark each case passed, failed, blocked or not exercised in the candidate's
private evidence report.

## Cases

| ID | Check | Required result |
|---|---|---|
| H01 | Load a synthetic old config without `[history]`; inspect a newly generated config separately | Old config stays off; new template opts in; explicit false survives unrelated saves |
| H02 | Enable, pause, resume and disable using the real CLI and a disposable config | Validated writes preserve unrelated keys/comments; readback agrees; next daemon session applies the change without restart |
| H03 | Drive delivered, no-speech, too-short, empty, timeout, cancelled, transcription-error and output-error daemon paths with synthetic capture/transcriber/output fixtures | Exactly one entry per completed session; no duplicate at repeated idle; correct outcome/error and `ended_by`; no history when off or paused |
| H04 | Inspect raw/processed/corrected text and runtime metadata on those entries | Original text and automatic edits preserved; actual engine/model/backend/language/timing/output information recorded where supplied; unknown values not invented from the binary's capabilities |
| H05 | Pause during a session and while its writer waits | Suppressed entry reaches neither day file nor index; queued-pause regression remains enforced |
| H06 | Simulate supported compositor replies for excluded apps at either end, missing/invalid replies and timeouts | Excluded entries absent; titles opt-in; unknown-app behavior matches documented default and does not claim exclusion protection |
| H07 | List/show/search with JSON, including a synthetic meeting match | Every subcommand accepts `--json`; stdout parses; errors have nonzero exits; one default search includes both sources |
| H08 | Copy and paste again through capture-only fixtures, including failed entries and empty corrections | Correct text and normal output hooks/fallback are used; paste delay observed; no new history entry created; no real clipboard or typing |
| H09 | Correct and delete a dictation, then search and rebuild | Original fields remain; intentional empty correction stays empty; deleted result disappears from day file and derived index; newer-schema correction fails without losing bytes |
| H10 | Test plain/gzipped days, corrupt lines, unterminated tails, retention 30 and 0 | Gzip remains readable/editable/searchable; unrelated corrupt bytes preserved; append stays parseable; retention removes complete old days; 0 keeps everything |
| H11 | Exercise private permissions, symlink boundaries, exclusive rewrite temporaries and a concurrent writer lock | No linked target is modified; private modes enforced; reopening cannot remove an active writer's temporary |
| H12 | Hold a SQLite reader while deleting, then release it and retry cleanup | Initial incomplete WAL cleanup is reported; retry succeeds; stale terms do not survive an unrelated corrupt day or missed append/delete synchronization |
| H13 | Exercise the real terminal reader through a private PTY and configurator through disposable file-layer state | Day groups, source/app/model/failure/correction markers, shared search, diff, row actions and off/paused states work; editing shortcuts do not escape the editor; settings persist without changing unrelated values |
| H14 | Exercise QML and GTK with real CLI calls on synthetic history | Day groups/search/expansion/copy/paste/correct/delete and enable/pause/resume are usable; invalid operations display errors; empty, failed, corrected, off and paused screenshots retained |
| H15 | Invoke tray and standalone/integrated deep links using the extracted candidate package | Correct frontend opens the history view; missing frontend fails or falls back honestly; actual helper/QML files are packaged at the versioned entry path; enable/live state/pause/copy actions preserve the config override |
| H16 | Run setup guidance and documented recovery commands on private fixtures | Upgrade enable hint and paused state are accurate; unknown-focus and deletion/WAL limits are disclosed; failed writes are not reported as successful |

## Automated starting points

```bash
cargo test --locked --no-default-features --lib history
cargo test --locked --no-default-features --test history_cli
cargo test --locked --no-default-features --test history_privacy
```

These commands are examples for the isolated approved builder, not permission
to run contributor tests on a working desktop. The filtered library command is
not the full regression suite. GTK/QML, terminal layout and extracted-package
checks require their own producer records; unit tests alone do not pass H13-H15.
