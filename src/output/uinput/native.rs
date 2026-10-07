use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use evdev::{
    uinput::VirtualDevice, AttributeSet, BusType, Device, InputEvent, InputId, KeyCode as Key,
    LedCode,
};
use xkbcommon::xkb::{self, compose};

use super::DEVICE_NAME;

pub(super) fn device() -> Arc<Mutex<Option<VirtualDevice>>> {
    static DEVICE: std::sync::OnceLock<Arc<Mutex<Option<VirtualDevice>>>> =
        std::sync::OnceLock::new();
    DEVICE.get_or_init(Default::default).clone()
}
use crate::config::OutputConfig;
use crate::error::OutputError;

const SHIFT: u16 = 42;
const ALTGR: u16 = 100;
const MODIFIERS: &[u16] = &[29, 97, 42, 54, 56, 100, 125, 126];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stroke {
    key: u16,
    modifiers: Vec<u16>,
}

type Plan = Vec<Vec<Stroke>>;

struct Planner {
    keymap: xkb::Keymap,
    compose: compose::Table,
    characters: BTreeMap<char, Vec<Stroke>>,
}

fn stopped(reason: impl Into<String>) -> OutputError {
    OutputError::KeyboardStopped(reason.into())
}

impl Planner {
    fn new(layout: &str, variant: &str, locale: &str) -> Result<Self, OutputError> {
        if layout.is_empty()
            || [layout, variant, locale].iter().any(|s| s.contains('\0'))
            || layout.contains(',')
            || variant.contains(',')
        {
            return Err(stopped(
                "configure one valid XKB layout and variant for the Voxtype keyboard",
            ));
        }
        let context = xkb::Context::new(xkb::CONTEXT_NO_ENVIRONMENT_NAMES);
        let keymap = xkb::Keymap::new_from_names(
            &context,
            "evdev",
            "pc105",
            layout,
            variant,
            Some(String::new()),
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .ok_or_else(|| {
            stopped(format!(
                "cannot compile XKB layout {layout:?}, variant {variant:?}"
            ))
        })?;
        let compose = compose::Table::new_from_locale(
            &context,
            std::ffi::OsStr::new(locale),
            compose::COMPILE_NO_FLAGS,
        )
        .map_err(|_| stopped(format!("cannot load compose table for locale {locale:?}")))?;
        let mut planner = Self {
            keymap,
            compose,
            characters: BTreeMap::new(),
        };
        planner.index_characters();
        Ok(planner)
    }

    fn index_characters(&mut self) {
        let mut state = xkb::State::new(&self.keymap);
        let altgr = state
            .key_get_one_sym(xkb::Keycode::new(u32::from(ALTGR) + 8))
            .raw()
            == xkb::keysyms::KEY_ISO_Level3_Shift;
        let mut combinations = vec![vec![], vec![SHIFT]];
        if altgr {
            combinations.extend([vec![ALTGR], vec![SHIFT, ALTGR]]);
        }
        let mut symbols: BTreeMap<u32, Stroke> = BTreeMap::new();
        for modifiers in combinations {
            for key in 1..=255_u16 {
                if MODIFIERS.contains(&key) || [1, 14, 15, 28, 58, 69, 70].contains(&key) {
                    continue;
                }
                for modifier in &modifiers {
                    state.update_key(
                        xkb::Keycode::new(u32::from(*modifier) + 8),
                        xkb::KeyDirection::Down,
                    );
                }
                let symbol = state.key_get_one_sym(xkb::Keycode::new(u32::from(key) + 8));
                for modifier in modifiers.iter().rev() {
                    state.update_key(
                        xkb::Keycode::new(u32::from(*modifier) + 8),
                        xkb::KeyDirection::Up,
                    );
                }
                let stroke = Stroke {
                    key,
                    modifiers: modifiers.clone(),
                };
                symbols.entry(symbol.raw()).or_insert(stroke);
            }
        }
        let mut composition = compose::State::new(&self.compose, compose::STATE_NO_FLAGS);
        for (&symbol, stroke) in &symbols {
            composition.reset();
            composition.feed(xkb::Keysym::new(symbol));
            if composition.status() == compose::Status::Nothing {
                if let Some(character) =
                    char::from_u32(xkb::keysym_to_utf32(xkb::Keysym::new(symbol)))
                {
                    self.insert(character, vec![stroke.clone()]);
                }
                continue;
            }
            if composition.status() != compose::Status::Composing {
                continue;
            }
            // Two-key dead-key sequences cover the common accented Latin
            // characters and literal dead-key punctuation. Longer compose
            // sequences are rejected rather than guessed or silently dropped.
            for (&second, second_stroke) in &symbols {
                composition.reset();
                composition.feed(xkb::Keysym::new(symbol));
                composition.feed(xkb::Keysym::new(second));
                if composition.status() != compose::Status::Composed {
                    continue;
                }
                if let Some(text) = composition.utf8() {
                    let mut chars = text.chars();
                    if let (Some(character), None) = (chars.next(), chars.next()) {
                        self.insert(character, vec![stroke.clone(), second_stroke.clone()]);
                    }
                }
            }
        }
    }

    fn insert(&mut self, character: char, strokes: Vec<Stroke>) {
        if character.is_control() {
            return;
        }
        let cost =
            |strokes: &[Stroke]| strokes.iter().map(|s| 1 + s.modifiers.len()).sum::<usize>();
        if self
            .characters
            .get(&character)
            .is_none_or(|old| cost(&strokes) < cost(old))
        {
            self.characters.insert(character, strokes);
        }
    }

    fn plan(&self, text: &str) -> Result<Plan, OutputError> {
        text.chars()
            .enumerate()
            .map(|(index, character)| {
                // A transcription line break must not execute a terminal command.
                let character = if matches!(character, '\n' | '\r') {
                    ' '
                } else {
                    character
                };
                self.characters.get(&character).cloned().ok_or_else(|| {
                    stopped(format!(
                        "unsupported character U+{:04X} at character {}; nothing was typed",
                        character as u32,
                        index + 1,
                    ))
                })
            })
            .collect()
    }
}

trait Sink {
    fn key(&mut self, key: u16, down: bool) -> std::io::Result<()>;
    fn pause(&mut self, duration: Duration);
}

fn dispatch(
    plan: &Plan,
    sink: &mut impl Sink,
    mut guard: impl FnMut() -> Result<(), OutputError>,
    delay: Duration,
) -> Result<(), OutputError> {
    let mut pressed = BTreeSet::new();
    let mut completed = 0;
    let result = (|| {
        for character in plan {
            for stroke in character {
                guard()?;
                for &key in stroke.modifiers.iter().chain(std::iter::once(&stroke.key)) {
                    // Track even a failed write: release is safe when the kernel
                    // accepted an event but the surrounding operation failed.
                    pressed.insert(key);
                    sink.key(key, true).map_err(|e| stopped(e.to_string()))?;
                    sink.pause(delay);
                }
                for &key in std::iter::once(&stroke.key).chain(stroke.modifiers.iter().rev()) {
                    sink.key(key, false).map_err(|e| stopped(e.to_string()))?;
                    pressed.remove(&key);
                    sink.pause(delay);
                }
            }
            completed += 1;
        }
        Ok(())
    })();
    for key in pressed {
        let _ = sink.key(key, false);
    }
    result.map_err(|error: OutputError| {
        let reason = match error {
            OutputError::KeyboardStopped(reason) => reason,
            other => other.to_string(),
        };
        stopped(format!(
            "after {completed} of {} characters: {reason}",
            plan.len(),
        ))
    })
}

struct KernelSink<'a>(&'a mut VirtualDevice);
impl Sink for KernelSink<'_> {
    fn key(&mut self, key: u16, down: bool) -> std::io::Result<()> {
        // emit appends SYN_REPORT, so press and release are distinct frames.
        self.0.emit(&[InputEvent::new(
            evdev::EventType::KEY.0,
            key,
            i32::from(down),
        )])
    }
    fn pause(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

struct Guard(Vec<Device>);
impl Guard {
    fn new() -> Result<Self, OutputError> {
        let devices: Vec<_> = evdev::enumerate()
            .map(|(_, device)| device)
            .filter(|device| {
                device.name() != Some(DEVICE_NAME)
                    && device.supported_keys().is_some_and(|keys| {
                        keys.contains(Key::KEY_A)
                            && keys.contains(Key::KEY_Z)
                            && keys.contains(Key::KEY_ENTER)
                    })
            })
            .collect();
        if devices.is_empty() {
            return Err(stopped(
                "no readable keyboard for modifier checks; check /dev/input permissions",
            ));
        }
        Ok(Self(devices))
    }

    fn check(&mut self) -> Result<(), OutputError> {
        for device in &mut self.0 {
            let keys = device
                .get_key_state()
                .map_err(|e| stopped(format!("cannot check keyboard state: {e}")))?;
            if MODIFIERS.iter().any(|key| keys.contains(Key::new(*key))) {
                return Err(stopped("a physical modifier is held"));
            }
            if device
                .supported_leds()
                .is_some_and(|leds| leds.contains(LedCode::LED_CAPSL))
            {
                let leds = device
                    .get_led_state()
                    .map_err(|e| stopped(format!("cannot check Caps Lock: {e}")))?;
                if leds.contains(LedCode::LED_CAPSL) {
                    return Err(stopped("Caps Lock is active; turn it off before dictating"));
                }
            }
        }
        Ok(())
    }

    fn wait(&mut self, timeout: Duration) -> Result<(), OutputError> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.check() {
                Ok(()) => return Ok(()),
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(_) => std::thread::sleep(Duration::from_millis(15)),
            }
        }
    }
}

fn create_device() -> Result<VirtualDevice, OutputError> {
    let keys: AttributeSet<Key> = (1..=255).map(Key::new).collect();
    let device = VirtualDevice::builder()
        .and_then(|builder| {
            builder
                .name(DEVICE_NAME)
                .input_id(InputId::new(BusType::BUS_USB, 0x5658, 0x5459, 1))
                .with_keys(&keys)
        })
        .and_then(|builder| builder.build())
        .map_err(|e| {
            stopped(format!(
                "cannot create keyboard on /dev/uinput: {e}; check uinput permissions"
            ))
        })?;
    // Keep the same device across output-chain reconstruction.
    // The first delivery gives the display server time to register it.
    std::thread::sleep(Duration::from_millis(750));
    Ok(device)
}

fn plan_text(config: &OutputConfig, text: &str) -> Result<Plan, OutputError> {
    let layout = config.uinput_xkb_layout.as_deref().ok_or_else(|| {
        stopped(
            "set output.uinput_xkb_layout to the layout assigned to the Voxtype virtual keyboard",
        )
    })?;
    let variant = config.uinput_xkb_variant.as_deref().unwrap_or("");
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "en_US.UTF-8".into());
    let planner = Planner::new(layout, variant, &locale)?;
    let text = format!("{}{}", text, config.append_text.as_deref().unwrap_or(""));
    let mut plan = planner.plan(&text)?;
    if config.auto_submit {
        plan.push(vec![Stroke {
            key: 28,
            modifiers: vec![],
        }]);
    }
    Ok(plan)
}

pub(super) async fn output(
    config: OutputConfig,
    device: Arc<Mutex<Option<VirtualDevice>>>,
    text: String,
    backspaces: Option<usize>,
) -> Result<(), OutputError> {
    tokio::task::spawn_blocking(move || {
        let plan = if let Some(count) = backspaces {
            vec![
                vec![Stroke {
                    key: 14,
                    modifiers: vec![]
                }];
                count
            ]
        } else {
            plan_text(&config, &text)?
        };
        if plan.is_empty() {
            return Ok(());
        }
        let mut guard = Guard::new()?;
        guard.wait(Duration::from_millis(config.modifier_release_timeout_ms))?;
        std::thread::sleep(Duration::from_millis(u64::from(config.pre_type_delay_ms)));
        let mut slot = device
            .lock()
            .map_err(|_| stopped("keyboard state lock was poisoned"))?;
        if slot.is_none() {
            *slot = Some(create_device()?);
        }
        let delay = Duration::from_millis(u64::from(config.type_delay_ms.max(5)));
        let result = dispatch(
            &plan,
            &mut KernelSink(slot.as_mut().unwrap()),
            || guard.check(),
            delay,
        );
        if result.is_err() {
            // Destroying the device also releases any keys whose explicit
            // release failed. Never reuse an uncertain modifier state.
            *slot = None;
        }
        result
    })
    .await
    .map_err(|error| stopped(format!("keyboard worker failed: {error}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Capture {
        events: Vec<(u16, bool)>,
        fail_at: Option<usize>,
        attempts: usize,
    }

    impl Sink for Capture {
        fn key(&mut self, key: u16, down: bool) -> std::io::Result<()> {
            let attempt = self.attempts;
            self.attempts += 1;
            if self.fail_at == Some(attempt) {
                return Err(std::io::Error::other("simulated device disconnect"));
            }
            self.events.push((key, down));
            Ok(())
        }
        fn pause(&mut self, _: Duration) {}
    }

    // Independent receiver: consume the actual press/release stream using
    // libxkbcommon state and compose processing, not the planner's reverse map.
    fn receive(planner: &Planner, events: &[(u16, bool)]) -> String {
        let mut state = xkb::State::new(&planner.keymap);
        let mut composition = compose::State::new(&planner.compose, compose::STATE_NO_FLAGS);
        let mut text = String::new();
        for &(key, down) in events {
            let keycode = xkb::Keycode::new(u32::from(key) + 8);
            state.update_key(
                keycode,
                if down {
                    xkb::KeyDirection::Down
                } else {
                    xkb::KeyDirection::Up
                },
            );
            if !down {
                continue;
            }
            composition.feed(state.key_get_one_sym(keycode));
            match composition.status() {
                compose::Status::Nothing => text.push_str(&state.key_get_utf8(keycode)),
                compose::Status::Composed => {
                    text.push_str(&composition.utf8().unwrap());
                    composition.reset();
                }
                compose::Status::Cancelled => panic!("generated sequence cancelled composition"),
                compose::Status::Composing => {}
            }
        }
        assert_ne!(
            composition.status(),
            compose::Status::Composing,
            "unfinished dead-key sequence"
        );
        text
    }

    fn check(layout: &str, variant: &str, input: &str, expected: &str) -> Capture {
        let planner = Planner::new(layout, variant, "en_US.UTF-8").unwrap();
        let plan = planner.plan(input).unwrap();
        let mut capture = Capture::default();
        dispatch(&plan, &mut capture, || Ok(()), Duration::ZERO).unwrap();
        assert_eq!(receive(&planner, &capture.events), expected);
        assert!(!capture
            .events
            .iter()
            .any(|(key, _)| [29, 97, 56, 125, 126, 28, 15].contains(key)));
        capture
    }

    #[test]
    fn every_printable_ascii_character_and_literal_escapes() {
        let ascii: String = (' '..='~').collect();
        let input = format!("{ascii} literal \\n \\t \\b \\\\ -filename");
        check("us", "", &input, &input);
        check("us", "intl", &input, &input);
    }

    #[test]
    fn english_and_german_on_us_international_and_german_layouts() {
        let text = "Hello! Grüße, äöü ÄÖÜ ß é è ê € @ # zY.";
        check("us", "intl", text, text);
        check("de", "", text, text);
    }

    #[test]
    fn line_breaks_are_spaces_and_never_return_keys() {
        check("us", "", "one\ntwo\r\nthree", "one two  three");
    }

    #[test]
    fn long_dictation_survives_the_receiver_without_character_loss() {
        let text = "Grüße! Literal \\n, äöü ß and €; Yz. ".repeat(250);
        check("us", "intl", &text, &text);
    }

    #[test]
    fn unsupported_tail_rejects_the_whole_transcript() {
        let planner = Planner::new("us", "", "en_US.UTF-8").unwrap();
        for suffix in ["ä", "한", "\t", "\u{1b}", "\u{0}"] {
            let error = planner.plan(&format!("valid prefix{suffix}")).unwrap_err();
            assert!(error.to_string().contains("nothing was typed"));
        }
    }

    #[test]
    fn append_text_is_preflighted_and_submit_is_explicit() {
        let mut config = OutputConfig {
            uinput_xkb_layout: Some("us".into()),
            append_text: Some("\t".into()),
            ..Default::default()
        };
        assert!(plan_text(&config, "hello").is_err());
        config.append_text = Some("!".into());
        let plan = plan_text(&config, "hello").unwrap();
        assert_eq!(plan.len(), 6);
        assert!(!plan.iter().flatten().any(|stroke| stroke.key == 28));
        config.auto_submit = true;
        let plan = plan_text(&config, "hello").unwrap();
        assert_eq!(
            plan.last().unwrap(),
            &[Stroke {
                key: 28,
                modifiers: vec![]
            }]
        );
        config.uinput_xkb_layout = None;
        assert!(plan_text(&config, "hello").is_err());
    }

    #[test]
    fn invalid_keymaps_fail_without_panicking_or_emitting() {
        for (layout, variant) in [
            ("", ""),
            ("us,de", ""),
            ("us\0", ""),
            ("us", "a\0"),
            ("missing-layout", ""),
        ] {
            assert!(Planner::new(layout, variant, "en_US.UTF-8").is_err());
        }
    }

    #[test]
    fn device_failure_releases_every_pressed_key_at_every_event_boundary() {
        let planner = Planner::new("us", "intl", "en_US.UTF-8").unwrap();
        let plan = planner.plan("Ä@'").unwrap();
        let mut baseline = Capture::default();
        dispatch(&plan, &mut baseline, || Ok(()), Duration::ZERO).unwrap();
        for index in 0..baseline.events.len() {
            let mut capture = Capture {
                fail_at: Some(index),
                ..Default::default()
            };
            assert!(dispatch(&plan, &mut capture, || Ok(()), Duration::ZERO).is_err());
            let mut held = BTreeSet::new();
            for (key, down) in capture.events {
                if down {
                    held.insert(key);
                } else {
                    held.remove(&key);
                }
            }
            assert!(held.is_empty(), "keys remained pressed after event {index}");
        }
    }

    #[test]
    fn modifier_change_aborts_before_the_next_stroke() {
        let planner = Planner::new("us", "", "en_US.UTF-8").unwrap();
        let plan = planner.plan("ABC").unwrap();
        let mut capture = Capture::default();
        let mut checks = 0;
        let result = dispatch(
            &plan,
            &mut capture,
            || {
                checks += 1;
                if checks == 2 {
                    Err(stopped("physical Ctrl pressed"))
                } else {
                    Ok(())
                }
            },
            Duration::ZERO,
        );
        assert!(result.is_err());
        assert_eq!(receive(&planner, &capture.events), "A");
    }
}
