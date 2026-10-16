//! Native X11 text output
//!
//! Types text into the focused window through the XTEST extension, giving
//! every character the keyboard layout cannot produce a keycode of its own.
//!
//! Why it is built this way: a character the keymap does not map cannot be
//! sent at all - XTEST presses keycodes, not characters - so it has to be bound
//! to a keycode first. Binding one keycode per character and pressing it
//! straight away loses or reorders characters, because clients resolve key
//! events through their own copy of the keyboard mapping and only refresh it
//! when the server tells them it changed. Measured on a GTK4 entry with a tool
//! that rebinds per character: 11 of 12 Han characters were lost at zero delay,
//! 3 of 12 at 2 ms, and the text arrived intact from 5 ms on. This driver binds
//! the distinct characters of a dictation into as few keymap changes as the
//! mapping's unused keycodes allow (one, for any realistic utterance) and waits
//! once for each change to settle, so the settle cost is paid per transcription
//! instead of per character.
//!
//! Requirements: an X server with XTEST (Xorg, XWayland). No external tool, no
//! daemon, and no libX11 at build or run time - the X protocol is spoken
//! directly over the socket.

use super::TextOutput;
use crate::error::OutputError;
use std::collections::HashMap;
use std::thread::sleep;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

/// The event codes XTEST's `fake_input` expects as its `type` argument.
const KEY_PRESS: u8 = 2;
const KEY_RELEASE: u8 = 3;

/// Keysyms worth naming rather than computing.
const KS_RETURN: u32 = 0xff0d;
const KS_TAB: u32 = 0xff09;
const KS_SHIFT_LEFT: u32 = 0xffe1;
const KS_SHIFT_RIGHT: u32 = 0xffe2;

/// Native X11 text output.
pub struct X11Output {
    /// Delay between key presses in milliseconds
    type_delay_ms: u32,
    /// Delay before typing starts in milliseconds
    pre_type_delay_ms: u32,
    /// How long to wait after changing the keyboard mapping before pressing
    /// the keys that depend on it, and again before restoring it
    keymap_settle_ms: u32,
    /// Whether to send Enter after the text
    auto_submit: bool,
    /// Text appended after the transcription (before auto_submit)
    append_text: Option<String>,
}

/// What to do for one character: which keycode to press, and whether Shift
/// reaches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stroke {
    keycode: u8,
    shift: bool,
}

/// What the driver needs to know to type one string.
#[derive(Debug, Clone, Copy)]
struct TypeOptions {
    type_delay_ms: u32,
    keymap_settle_ms: u32,
    auto_submit: bool,
}

/// Characters the current keyboard mapping can already produce, and how.
struct Layout {
    by_char: HashMap<char, Stroke>,
}

impl Layout {
    /// Index a server keyboard mapping: level 0 is a key's unshifted keysym,
    /// level 1 its shifted one.
    ///
    /// Higher levels (AltGr) are deliberately ignored. A character reachable
    /// only through AltGr falls through to the generated mapping instead, where
    /// it gets a keycode of its own and needs no modifier at all.
    fn from_keysyms(keysyms: &[u32], per_keycode: usize, min_keycode: u8) -> Self {
        let mut by_char = HashMap::new();
        if per_keycode == 0 {
            return Self { by_char };
        }
        for (index, level) in keysyms.chunks(per_keycode).enumerate() {
            let keycode = min_keycode.wrapping_add(index as u8);
            for (shift, keysym) in [false, true].into_iter().zip(level.iter().take(2)) {
                if let Some(ch) = char_for_keysym(*keysym) {
                    by_char.entry(ch).or_insert(Stroke { keycode, shift });
                }
            }
        }
        Self { by_char }
    }

    fn stroke_for(&self, ch: char) -> Option<Stroke> {
        self.by_char.get(&ch).copied()
    }

    /// The keycode carrying `wanted`, for keys that are pressed as themselves
    /// rather than for the character they type (Shift, Return).
    fn keycode_for_keysym(
        keysyms: &[u32],
        per_keycode: usize,
        min_keycode: u8,
        wanted: u32,
    ) -> Option<u8> {
        if per_keycode == 0 {
            return None;
        }
        keysyms
            .chunks(per_keycode)
            .position(|level| level.contains(&wanted))
            .map(|index| min_keycode.wrapping_add(index as u8))
    }
}

/// Runs of keycodes no level of the current mapping binds to anything, and
/// which are therefore free to point at a generated character mapping.
///
/// Each run is contiguous and ascending, because one `ChangeKeyboardMapping`
/// request writes an unbroken range. Runs are ordered longest first, and
/// highest first among equals, so the common case needs a single mapping change
/// and stays as far as possible from keycodes a physical keyboard might send.
fn spare_keycode_runs(keysyms: &[u32], per_keycode: usize, min_keycode: u8) -> Vec<Vec<u8>> {
    if per_keycode == 0 {
        return Vec::new();
    }
    let mut runs: Vec<Vec<u8>> = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    for (index, level) in keysyms.chunks(per_keycode).enumerate() {
        let keycode = min_keycode.wrapping_add(index as u8);
        if level.iter().all(|keysym| *keysym == 0) {
            current.push(keycode);
        } else if !current.is_empty() {
            runs.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs.sort_by(|a, b| b.len().cmp(&a.len()).then(b[0].cmp(&a[0])));
    runs
}

/// The keysym that types `ch`, if the X protocol has one.
fn keysym_for(ch: char) -> Option<u32> {
    match ch {
        '\n' | '\r' => Some(KS_RETURN),
        '\t' => Some(KS_TAB),
        // Control characters other than the named ones have no keysym that
        // types them.
        c if (c as u32) < 0x20 => None,
        // Latin-1 keysyms are the code point itself; everything above it lives
        // in the Unicode keysym range of 0x01000000 + code point.
        c if (c as u32) <= 0xff => Some(c as u32),
        c => Some(0x0100_0000 | c as u32),
    }
}

/// The character a keysym types, if it types one.
fn char_for_keysym(keysym: u32) -> Option<char> {
    match keysym {
        0 => None,
        KS_RETURN => Some('\n'),
        KS_TAB => Some('\t'),
        0x20..=0xff => char::from_u32(keysym),
        0x0100_0000..=0x0110_ffff => char::from_u32(keysym - 0x0100_0000),
        _ => None,
    }
}

/// One keymap change plus the keystrokes that depend on it.
#[derive(Debug, Default, PartialEq, Eq)]
struct Batch {
    /// keycode -> keysym to bind before pressing `strokes`, ascending keycodes
    /// from one contiguous run
    remaps: Vec<(u8, u32)>,
    /// The characters to press, in the order they appear in the text, with the
    /// layout's own keycodes for characters it can already produce
    strokes: Vec<Stroke>,
}

/// Split `text` into batches, binding every character the layout lacks to a
/// keycode of its own.
///
/// Characters the layout already produces are pressed as they are; a character
/// that needs a binding keeps it for the rest of its batch. Returns the batches
/// and the number of characters no keycode was left to bind (which the caller
/// reports rather than typing the text partially).
fn plan(text: &str, layout: &Layout, runs: &[Vec<u8>]) -> (Vec<Batch>, usize) {
    let mut batches: Vec<Batch> = Vec::new();
    let mut batch = Batch::default();
    let mut bound: HashMap<u32, u8> = HashMap::new();
    let mut run_index = 0usize;
    let mut unbound = 0usize;

    for ch in text.chars() {
        if let Some(stroke) = layout.stroke_for(ch) {
            batch.strokes.push(stroke);
            continue;
        }
        let Some(keysym) = keysym_for(ch) else {
            continue;
        };
        if let Some(keycode) = bound.get(&keysym) {
            batch.strokes.push(Stroke {
                keycode: *keycode,
                shift: false,
            });
            continue;
        }
        if runs.is_empty() {
            unbound += 1;
            continue;
        }
        // Advance to a run with a free slot, flushing what is queued: one
        // request covers one run, so a batch never straddles two. Wrapping
        // around reuses the runs for a later batch, so the text a driver can
        // type is not capped by how many keycodes the mapping leaves unused.
        while batch.remaps.len() >= runs[run_index].len() {
            if !batch.strokes.is_empty() {
                batches.push(std::mem::take(&mut batch));
            }
            bound.clear();
            run_index = (run_index + 1) % runs.len();
        }
        let keycode = runs[run_index][batch.remaps.len()];
        batch.remaps.push((keycode, keysym));
        bound.insert(keysym, keycode);
        batch.strokes.push(Stroke {
            keycode,
            shift: false,
        });
    }

    if !batch.strokes.is_empty() {
        batches.push(batch);
    }
    (batches, unbound)
}

/// Connect to the display named by `$DISPLAY`, and require XTEST to be there.
fn connect_with_xtest() -> Result<RustConnection, OutputError> {
    let (conn, _screen_num) =
        x11rb::connect(None).map_err(|e| OutputError::X11DisplayUnavailable(e.to_string()))?;
    let present = conn
        .query_extension(b"XTEST")
        .map_err(|e| OutputError::X11DisplayUnavailable(e.to_string()))?
        .reply()
        .map_err(|e| OutputError::X11DisplayUnavailable(e.to_string()))?
        .present;
    if !present {
        return Err(OutputError::X11XtestMissing);
    }
    Ok(conn)
}

fn fake_key(conn: &RustConnection, keycode: u8, press: bool) -> Result<(), OutputError> {
    conn.xtest_fake_input(
        if press { KEY_PRESS } else { KEY_RELEASE },
        keycode,
        x11rb::CURRENT_TIME,
        x11rb::NONE,
        0,
        0,
        0,
    )
    .map_err(|e| OutputError::InjectionFailed(format!("XTEST key event failed: {e}")))?;
    Ok(())
}

/// Press and release one key, holding Shift around it when the character needs
/// the shifted level of its keycode.
fn press_key(conn: &RustConnection, stroke: Stroke, shift: Option<u8>) -> Result<(), OutputError> {
    let shift = if stroke.shift { shift } else { None };
    if let Some(shift) = shift {
        fake_key(conn, shift, true)?;
    }
    fake_key(conn, stroke.keycode, true)?;
    fake_key(conn, stroke.keycode, false)?;
    if let Some(shift) = shift {
        fake_key(conn, shift, false)?;
    }
    Ok(())
}

/// Force a round trip, so everything sent so far has been processed.
fn sync(conn: &RustConnection) -> Result<(), OutputError> {
    conn.flush()
        .map_err(|e| OutputError::InjectionFailed(format!("X11 flush failed: {e}")))?;
    conn.get_input_focus()
        .map_err(|e| OutputError::InjectionFailed(format!("X11 sync failed: {e}")))?
        .reply()
        .map_err(|e| OutputError::InjectionFailed(format!("X11 sync failed: {e}")))?;
    Ok(())
}

/// Fold any X11 error from a key-state query into the output error type.
fn key_state_error<E: std::fmt::Display>(e: E) -> OutputError {
    OutputError::InjectionFailed(format!("X11 key state query failed: {e}"))
}

/// Keycodes currently held down that belong to a modifier, so that a
/// transcription cannot combine with a held hotkey chord (Meta+V, say) into an
/// application shortcut.
fn held_modifier_keycodes(conn: &RustConnection) -> Result<Vec<u8>, OutputError> {
    let pressed = conn
        .query_keymap()
        .map_err(key_state_error)?
        .reply()
        .map_err(key_state_error)?
        .keys;
    let modifier_keycodes = conn
        .get_modifier_mapping()
        .map_err(key_state_error)?
        .reply()
        .map_err(key_state_error)?
        .keycodes;

    let mut held: Vec<u8> = modifier_keycodes
        .into_iter()
        .filter(|keycode| *keycode != 0)
        .filter(|keycode| pressed[(*keycode / 8) as usize] & (1 << (*keycode % 8)) != 0)
        .collect();
    held.sort_unstable();
    held.dedup();
    Ok(held)
}

/// Bind `batch.remaps` into the server's keyboard mapping.
fn apply_remaps(
    conn: &RustConnection,
    remaps: &[(u8, u32)],
    per_keycode: usize,
) -> Result<(), OutputError> {
    let keysyms: Vec<u32> = remaps
        .iter()
        .flat_map(|(_, keysym)| std::iter::repeat_n(*keysym, per_keycode))
        .collect();
    conn.change_keyboard_mapping(remaps.len() as u8, remaps[0].0, per_keycode as u8, &keysyms)
        .map_err(|e| OutputError::InjectionFailed(format!("X11 keymap change failed: {e}")))?;
    Ok(())
}

/// Put the originally reported keysyms back for every keycode we bound.
fn restore_keymap(
    conn: &RustConnection,
    batches: &[Batch],
    original: &[u32],
    per_keycode: usize,
    min_keycode: u8,
) -> Result<(), OutputError> {
    for batch in batches {
        if batch.remaps.is_empty() {
            continue;
        }
        let keysyms: Vec<u32> = batch
            .remaps
            .iter()
            .flat_map(|(keycode, _)| {
                let offset = (keycode - min_keycode) as usize * per_keycode;
                original[offset..offset + per_keycode].to_vec()
            })
            .collect();
        conn.change_keyboard_mapping(
            batch.remaps.len() as u8,
            batch.remaps[0].0,
            per_keycode as u8,
            &keysyms,
        )
        .map_err(|e| OutputError::InjectionFailed(format!("X11 keymap restore failed: {e}")))?;
    }
    Ok(())
}

/// Type `text` into the focused window. Blocking; call it from a blocking task.
fn type_text(text: &str, options: TypeOptions) -> Result<(), OutputError> {
    let conn = connect_with_xtest()?;
    let setup = conn.setup();
    let (min_keycode, max_keycode) = (setup.min_keycode, setup.max_keycode);

    let reply = conn
        .get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)
        .map_err(|e| OutputError::InjectionFailed(format!("X11 keymap read failed: {e}")))?
        .reply()
        .map_err(|e| OutputError::InjectionFailed(format!("X11 keymap read failed: {e}")))?;
    let per_keycode = reply.keysyms_per_keycode as usize;
    let original = reply.keysyms;

    let layout = Layout::from_keysyms(&original, per_keycode, min_keycode);
    let runs = spare_keycode_runs(&original, per_keycode, min_keycode);
    let (batches, unbound) = plan(text, &layout, &runs);

    if unbound > 0 {
        return Err(OutputError::InjectionFailed(format!(
            "no unused keycodes in the X11 keyboard mapping to bind {unbound} character(s) to"
        )));
    }
    if batches.is_empty() && !options.auto_submit {
        return Ok(());
    }

    let needs_remap = batches.iter().any(|batch| !batch.remaps.is_empty());

    let shift_keycode =
        Layout::keycode_for_keysym(&original, per_keycode, min_keycode, KS_SHIFT_LEFT).or_else(
            || Layout::keycode_for_keysym(&original, per_keycode, min_keycode, KS_SHIFT_RIGHT),
        );

    // Release held modifiers for the duration, then put them back: a
    // transcription typed while the hotkey chord is still down would otherwise
    // arrive as application shortcuts.
    let held = held_modifier_keycodes(&conn)?;
    for keycode in &held {
        fake_key(&conn, *keycode, false)?;
    }

    let settle = Duration::from_millis(options.keymap_settle_ms as u64);
    let pace = Duration::from_millis(options.type_delay_ms as u64);
    let last_batch = batches.len().saturating_sub(1);

    for (index, batch) in batches.iter().enumerate() {
        if !batch.remaps.is_empty() {
            apply_remaps(&conn, &batch.remaps, per_keycode)?;
            sync(&conn)?;
            // Clients resolve keycodes through their own copy of the keyboard
            // mapping, refreshed on a keymap-change notification. Pressing a
            // remapped keycode before they have caught up loses or swaps the
            // character.
            sleep(settle);
        }
        for stroke in &batch.strokes {
            press_key(&conn, *stroke, shift_keycode)?;
            if options.type_delay_ms > 0 {
                sleep(pace);
            }
        }
        sync(&conn)?;
        if index < last_batch {
            // The next batch rebinds these keycodes; let this batch's keys be
            // resolved against the mapping they were pressed with.
            sleep(settle);
        }
    }

    if needs_remap && !batches.is_empty() {
        // Wait for the last keys to be resolved before taking their keycodes
        // away again.
        sleep(settle);
        restore_keymap(&conn, &batches, &original, per_keycode, min_keycode)?;
        sync(&conn)?;
        sleep(settle);
    }

    for keycode in held.iter().rev() {
        fake_key(&conn, *keycode, true)?;
    }

    if options.auto_submit {
        if let Some(return_key) =
            Layout::keycode_for_keysym(&original, per_keycode, min_keycode, KS_RETURN)
        {
            press_key(
                &conn,
                Stroke {
                    keycode: return_key,
                    shift: false,
                },
                shift_keycode,
            )?;
        }
    }

    sync(&conn)?;
    Ok(())
}

impl X11Output {
    /// Create a new native X11 output
    pub fn new(
        type_delay_ms: u32,
        pre_type_delay_ms: u32,
        keymap_settle_ms: u32,
        auto_submit: bool,
        append_text: Option<String>,
    ) -> Self {
        Self {
            type_delay_ms,
            pre_type_delay_ms,
            keymap_settle_ms,
            auto_submit,
            append_text,
        }
    }

    async fn type_text_async(&self, text: &str) -> Result<(), OutputError> {
        let options = TypeOptions {
            type_delay_ms: self.type_delay_ms,
            keymap_settle_ms: self.keymap_settle_ms,
            auto_submit: self.auto_submit,
        };
        let payload = text.to_string();
        tokio::task::spawn_blocking(move || type_text(&payload, options))
            .await
            .map_err(|e| OutputError::InjectionFailed(format!("X11 typing task failed: {e}")))?
    }
}

#[async_trait::async_trait]
impl TextOutput for X11Output {
    async fn output(&self, text: &str) -> Result<(), OutputError> {
        if text.is_empty() && self.append_text.is_none() && !self.auto_submit {
            return Ok(());
        }

        if self.pre_type_delay_ms > 0 {
            tracing::debug!("x11: sleeping {}ms before typing", self.pre_type_delay_ms);
            tokio::time::sleep(Duration::from_millis(self.pre_type_delay_ms as u64)).await;
        }

        let mut payload = text.to_string();
        if let Some(append) = &self.append_text {
            payload.push_str(append);
        }

        self.type_text_async(&payload).await?;

        tracing::info!("Text typed via x11 ({} chars)", payload.chars().count());
        Ok(())
    }

    async fn is_available(&self) -> bool {
        tokio::task::spawn_blocking(|| connect_with_xtest().is_ok())
            .await
            .unwrap_or(false)
    }

    fn name(&self) -> &'static str {
        "x11"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin1_and_unicode_keysyms() {
        assert_eq!(keysym_for('a'), Some(0x61));
        assert_eq!(keysym_for(' '), Some(0x20));
        assert_eq!(keysym_for('é'), Some(0xe9));
        assert_eq!(keysym_for('中'), Some(0x0100_4e2d));
        assert_eq!(keysym_for('🦜'), Some(0x0101_f99c));
        assert_eq!(keysym_for('\n'), Some(KS_RETURN));
        assert_eq!(keysym_for('\t'), Some(KS_TAB));
        assert_eq!(keysym_for('\u{7}'), None);
    }

    #[test]
    fn keysyms_round_trip_to_characters() {
        for ch in ['a', 'é', '中', '🦜'] {
            assert_eq!(char_for_keysym(keysym_for(ch).unwrap()), Some(ch));
        }
        assert_eq!(char_for_keysym(0), None);
        assert_eq!(char_for_keysym(0xff0d), Some('\n'));
        assert_eq!(char_for_keysym(KS_SHIFT_LEFT), None);
    }

    #[test]
    fn layout_indexes_unshifted_and_shifted_levels() {
        let layout = Layout::from_keysyms(&[0x61, 0x41, 0, 0], 2, 8);
        assert_eq!(
            layout.stroke_for('a'),
            Some(Stroke {
                keycode: 8,
                shift: false
            })
        );
        assert_eq!(
            layout.stroke_for('A'),
            Some(Stroke {
                keycode: 8,
                shift: true
            })
        );
        assert_eq!(layout.stroke_for('中'), None);
    }

    #[test]
    fn layout_finds_named_keycodes() {
        // keycodes 8 = a, 9 = Shift_L
        let keysyms = [0x61, 0x41, 0xffe1, 0, 0, 0];
        assert_eq!(
            Layout::keycode_for_keysym(&keysyms, 2, 8, KS_SHIFT_LEFT),
            Some(9)
        );
        assert_eq!(Layout::keycode_for_keysym(&keysyms, 2, 8, KS_RETURN), None);
    }

    #[test]
    fn spare_keycode_runs_group_contiguous_keycodes() {
        // per_keycode 2, min 8: keycodes 8, 9 bound; 10, 11 free
        let runs = spare_keycode_runs(&[0x61, 0, 0x62, 0, 0, 0, 0, 0], 2, 8);
        assert_eq!(runs, vec![vec![10, 11]]);
    }

    #[test]
    fn spare_keycode_runs_prefer_long_runs_and_stay_contiguous() {
        // keycodes: 8 bound, 9 free, 10 bound, 11..13 free
        let keysyms = [0x61, 0, 0, 0, 0x62, 0, 0, 0, 0, 0, 0, 0];
        let runs = spare_keycode_runs(&keysyms, 2, 8);
        assert_eq!(runs, vec![vec![11, 12, 13], vec![9]]);
    }

    #[test]
    fn plan_reuses_layout_characters_without_remapping() {
        let layout = Layout::from_keysyms(&[0x61, 0, 0, 0], 2, 8);
        let (batches, unbound) = plan("ab", &layout, &[vec![10, 11]]);
        assert_eq!(unbound, 0);
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0].strokes,
            vec![
                Stroke {
                    keycode: 8,
                    shift: false
                },
                Stroke {
                    keycode: 10,
                    shift: false
                },
            ]
        );
        assert_eq!(batches[0].remaps, vec![(10, 0x62)]);
    }

    #[test]
    fn plan_binds_each_distinct_character_once_and_reuses_repeats() {
        let layout = Layout::from_keysyms(&[], 2, 8);
        let (batches, unbound) = plan("中中", &layout, &[vec![10, 11]]);
        assert_eq!(unbound, 0);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].remaps, vec![(10, 0x0100_4e2d)]);
        assert_eq!(batches[0].strokes.len(), 2);
        assert_eq!(batches[0].strokes[0], batches[0].strokes[1]);
    }

    #[test]
    fn plan_keeps_a_whole_run_in_one_contiguous_batch() {
        let layout = Layout::from_keysyms(&[], 2, 8);
        let (batches, unbound) = plan("中文", &layout, &[vec![11, 12]]);
        assert_eq!(unbound, 0);
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0].remaps,
            vec![(11, 0x0100_4e2d), (12, 0x0100_6587)]
        );
    }

    #[test]
    fn plan_splits_across_runs_keeping_each_run_contiguous() {
        let layout = Layout::from_keysyms(&[], 2, 8);
        let (batches, unbound) = plan("中文", &layout, &[vec![11], vec![9]]);
        assert_eq!(unbound, 0);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].remaps, vec![(11, 0x0100_4e2d)]);
        assert_eq!(
            batches[0].strokes,
            vec![Stroke {
                keycode: 11,
                shift: false
            }]
        );
        assert_eq!(batches[1].remaps, vec![(9, 0x0100_6587)]);
        assert_eq!(
            batches[1].strokes,
            vec![Stroke {
                keycode: 9,
                shift: false
            }]
        );
    }

    #[test]
    fn plan_reuses_a_run_for_later_characters() {
        let layout = Layout::from_keysyms(&[], 2, 8);
        let (batches, unbound) = plan("中文", &layout, &[vec![10]]);
        assert_eq!(unbound, 0);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].remaps, vec![(10, 0x0100_4e2d)]);
        assert_eq!(batches[1].remaps, vec![(10, 0x0100_6587)]);
    }

    #[test]
    fn plan_wraps_around_when_text_outlasts_the_runs() {
        let layout = Layout::from_keysyms(&[], 2, 8);
        let text: String = (0..5)
            .map(|i| char::from_u32(0x4e00 + i).unwrap())
            .collect();
        let (batches, unbound) = plan(&text, &layout, &[vec![10, 11]]);
        assert_eq!(unbound, 0);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].remaps.len(), 2);
        assert_eq!(batches[1].remaps.len(), 2);
        assert_eq!(batches[2].remaps.len(), 1);
        // the third batch reuses the first run's keycodes
        assert_eq!(batches[0].remaps[0].0, batches[2].remaps[0].0);
    }

    #[test]
    fn plan_with_no_spare_keycodes_binds_nothing() {
        let layout = Layout::from_keysyms(&[], 2, 8);
        let (batches, unbound) = plan("中", &layout, &[]);
        assert!(batches.is_empty());
        assert_eq!(unbound, 1);
    }

    #[test]
    fn plan_skips_characters_with_no_keysym() {
        let layout = Layout::from_keysyms(&[], 2, 8);
        let (batches, unbound) = plan("\u{7}", &layout, &[vec![10]]);
        assert!(batches.is_empty());
        assert_eq!(unbound, 0);
    }

    #[test]
    fn mixed_text_keeps_order() {
        let layout = Layout::from_keysyms(&[0x61, 0, 0, 0], 2, 8);
        let (batches, unbound) = plan("a中b", &layout, &[vec![10, 11]]);
        assert_eq!(unbound, 0);
        assert_eq!(
            batches[0].strokes,
            vec![
                Stroke {
                    keycode: 8,
                    shift: false
                },
                Stroke {
                    keycode: 10,
                    shift: false
                },
                Stroke {
                    keycode: 11,
                    shift: false
                },
            ]
        );
        assert_eq!(batches[0].remaps, vec![(10, 0x0100_4e2d), (11, 0x62)]);
    }
}
