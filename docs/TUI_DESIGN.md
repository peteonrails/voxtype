# `voxtype configure` design

The design contract for the configuration TUI. It covers how the interface
looks and behaves, not which settings exist; those live in the page modules
under `src/tui/` and in [CONFIGURATION.md](CONFIGURATION.md). Change this
file together with the code when the interaction changes.

## Promise

Open it, change what you came to change, press `s`, and voxtype is using it.
Most people only ever need the first page. Everything else is one step away
and never in the way.

## Principles

1. **One thing on screen per job.** A page shows its settings and a short
   explanation of the one under the cursor. No banners, legends, counters or
   duplicate status.
2. **Saving is applying.** voxtype reads its settings only when it starts,
   so `s` saves every change and restarts voxtype in the same step. There is
   no separate "apply", no restart prompt, and no message that tells you to
   restart something.
3. **Show the choices.** A setting with a handful of values shows all of
   them on its row, with the current one marked. Longer lists show the
   current value between `‹ ›` and step with the arrows.
4. **Plain words.** Settings are named for what they do ("Voice detection",
   "On-screen display"), values read as words ("Hold to talk"), and nothing
   mentions Cargo features, symlinks or file formats.
5. **Every instruction is something you can do here**, or an exact command
   to paste. Never "restart X" without saying how.
6. **Quiet by default.** State appears in two places only: a `●` beside each
   changed setting, and one status line.

## Pages

| Page | What is on it |
|---|---|
| **Basics** | Engine, model, hotkey, hotkey mode, output, on-screen display. Problems that stop voxtype from working (wrong build for the engine, model not downloaded) appear here as rows you can fix with Enter. |
| Engine | The engine's model and tuning (language, threads, loading). |
| Hotkey | Built-in listener, hotkey, mode, cancel key, modifier. |
| Audio | Input device, recording limit, media pausing, sounds. |
| Voice detection | Whether silence is trimmed or discarded, and how. |
| Output | How text is delivered, delays, post-processing. |
| Text | Spoken punctuation and word replacements. |
| Notifications | Desktop notifications. |
| On-screen display | The recording overlay: style, position, size. |
| Waybar | Status bar icons. |
| Meeting | Meeting transcription. |
| Advanced | Eager processing, GPU device, streaming. |
| Build | Which voxtype build is installed (CPU or GPU, Whisper or the others). |

Basics edits the same settings as the other pages, not copies, so a change
made on either shows on both. Each setting lives in exactly one page besides
Basics; nothing is duplicated between pages.

## Screen

```text
 voxtype           ╭ Basics ──────────────────────────────────────────────╮
                   │                                                      │
 › Basics          │ › Engine            ‹ Parakeet ›                     │
                   │   Model               parakeet-tdt-0.6b-v3           │
   Engine          │   Hotkey              SCROLLLOCK                     │
   Hotkey          │   Hotkey mode       ● Hold to talk  Press to toggle  │
   Audio           │   Output              Type  Paste  Clipboard  File   │
   ...             │   On-screen display   On  Off                        │
                   │                                                      │
                   │   Turns speech into text. Parakeet is fast and       │
                   │   accurate for English and 25 European languages.    │
                   ╰──────────────────────────────────────────────────────╯
 ● 1 unsaved change · s saves and restarts voxtype
 ↑↓ move   ←→ change   Enter edit   s save   Esc back   ? keys
```

- **Page list**: the product name, then the pages. Blank lines group them
  (Basics; the dictation path; status around it; the rest). No box, no
  headings. `›` marks the open page; while the list has focus the page under
  the cursor is reversed.
- **Page**: one rounded pane titled with the page name, its border in the
  accent color when it has focus. Settings first, then a blank line, then
  two or three lines about the setting under the cursor (PgUp/PgDn scroll
  longer text). The settings scroll when they do not fit.
- **Row**: cursor `›`, label, unsaved `●`, value. The focused row's label is
  bold in the accent color; its value shows how to change it (`‹ ›`, the
  marked choice, or a caret while typing). Settings that do nothing in the
  current setup are dimmed.
- **Status line**, in priority order: the result of the last save
  (`✓`/`✗`), a section's own message, the unsaved count with the key that
  saves, otherwise what voxtype is running and which file this edits.
- **Footer**: at most six key hints for whatever has focus. Everything else
  is under `?`.

## Interaction

- Focus is either the page list or the page. Tab switches; Enter or → in
  the list opens a page; Esc in a page goes back to the list; Esc or `q` in
  the list quits.
- In a page: ↑↓ move, ←→ (or Space) change, Enter types a value, or advances
  a value that can't be typed. While typing, every key goes to the field.
- `s` saves all unsaved changes on every page, downloads a newly chosen
  model or switches build if the change needs it, and then restarts voxtype
  when it is running from the file being edited. A stopped voxtype is left
  stopped; the status line says it will use the settings when it starts.
- `r` puts the open page back to what is saved. Listed under `?` only.
- Quitting with unsaved changes asks once: save, discard, or keep editing.
  It is the only dialog.
- Below 60x16 the TUI asks for more room instead of drawing a broken
  screen.

## Copy rules

- Labels are nouns a user would say. The same setting has the same label on
  Basics and on its own page.
- Values are words, capitalized: `On`/`Off`, `Hold to talk`, `Not set`.
  The file keeps its own spelling (`true`, `push_to_talk`).
- Explanations are one or two sentences: what it does, then when to change
  it. Longer background belongs in the docs.
- Messages lead with the outcome, then the next step, and fit one line at
  80 columns.
- No em-dashes, no "please", no exclamation marks.

## Color and spacing

- Only the terminal's palette: default foreground and background, the 16
  ANSI colors, bold, dim and reverse. No RGB, no hard-coded white or black,
  so light and dark themes both restyle it.
- Muted text uses dim, never ANSI "bright black", which some themes set to
  the background.
- Green is the one accent: focus border, cursor, current choice, keys.
  Yellow means unsaved or needs attention, red a failure, cyan a
  recommendation.
- Color never carries meaning alone: `›` for the cursor, `●` for unsaved,
  `✓ ✗ !` on messages, `‹ ›` and reverse video on the current choice.
- One space of padding inside panes, one blank line between settings and
  their explanation, no rules or separators beyond the pane border.
