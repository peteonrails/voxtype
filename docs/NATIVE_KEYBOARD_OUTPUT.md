# Native keyboard output

The experimental Linux `uinput` driver types through a persistent virtual
keyboard. It does not read or write the clipboard, invoke paste shortcuts, or
use an X11 or Wayland injection protocol. Applications receive keyboard events
through the same input path as a hardware keyboard.

This driver is opt-in and requires a binary built with `--features uinput`.
Building requires libxkbcommon development files; running requires XKB keymap
and compose data, access to `/dev/uinput`, and permission to read keyboard
state from `/dev/input/event*`. Grant these permissions through your existing
input-device policy. Do not run the whole daemon as root.

## Configuration

```toml
[output]
mode = "type"
driver_order = ["uinput"]
fallback_to_clipboard = false
uinput_xkb_layout = "us"
uinput_xkb_variant = "intl"
auto_submit = false
type_delay_ms = 5
```

`uinput_xkb_layout` is required. `uinput_xkb_variant` defaults to an empty
variant. Only one layout is supported, with no XKB options. The desktop must
assign exactly this layout and variant to **Voxtype virtual keyboard**.
Voxtype cannot discover or control the recipient's mapping across every Linux
compositor. A mismatched map produces incorrect characters even when all
keyboard events are delivered successfully.

The driver must be the only entry in `driver_order`. Mixed chains are rejected.
Native errors never fall back to clipboard or another driver, including when
`fallback_to_clipboard` retains its legacy default of `true`.

CLI overrides are `--driver uinput`, `--uinput-xkb-layout`, and
`--uinput-xkb-variant`. Environment overrides are `VOXTYPE_UINPUT_XKB_LAYOUT`
and `VOXTYPE_UINPUT_XKB_VARIANT`.

## Desktop and remapper setup

The virtual keyboard has vendor ID `5658` and product ID `5459` (hexadecimal).
It is created lazily on first output and retained across subsequent output
chains. The first output waits 750 ms for device registration; this is a grace
period, not acknowledgement from the compositor.

With keyd, exclude it from a wildcard remapping configuration:

```ini
[ids]
*
-5658:5459
```

Keep physical X-keys devices in your existing remapping configuration. Their
recording trigger continues to use the existing hotkey path. Voxtype's evdev
hotkey listener ignores its own virtual keyboard to prevent feedback.
Other remappers need an equivalent exclusion; applying remapping twice can
change or consume dictated characters.

For Xorg, a matching InputClass can assign the map:

```conf
Section "InputClass"
    Identifier "Voxtype keyboard"
    MatchProduct "Voxtype virtual keyboard"
    Option "XkbLayout" "us"
    Option "XkbVariant" "intl"
    Option "XkbOptions" ""
EndSection
```

For Sway, find the device identifier using `swaymsg -t get_inputs` after the
device exists, then configure that identifier:

```conf
input "<Voxtype device identifier>" {
    xkb_layout us
    xkb_variant intl
    xkb_options ""
}
```

Other compositors require their own equivalent device configuration. If a
compositor cannot assign a compatible mapping, this driver cannot guarantee
correct text there. The kernel interface also reaches Linux virtual consoles,
but their console keymaps and compose handling differ from XKB; accented text
on a console is not covered by the XKB mapping guarantee.

## Text delivery and errors

The whole transcript, including `append_text`, is planned before opening an
input device or emitting events. Printable characters use direct keys or
supported two-key compose/dead-key sequences. Unsupported characters reject
the entire burst with their Unicode code point and position. Arbitrary Unicode
and longer compose sequences are not supported. The compose table follows
`LC_ALL`, `LC_CTYPE`, or `LANG`, with an `en_US.UTF-8` fallback; the recipient
must use compatible compose rules.

Literal backslashes and supported typographic punctuation are preserved.
Line-feed and carriage-return characters each become a space, so transcription
line breaks cannot submit terminal commands. Tabs and other control characters
are rejected. Only explicit `auto_submit = true` appends an Enter key.

The driver waits for physical modifiers to be released, refuses active Caps
Lock, and checks keyboard state before each stroke. These checks remain active
when the generic modifier guard is disabled. A delivery failure stops the
burst, attempts to release every pressed key, and destroys the device before
another delivery. Errors can follow partial insertion: do not blindly retry.
Streaming corrections and cancellation use the same virtual keyboard for
Backspace instead of switching to an external typing tool.

At least 5 ms separates key events, including modifier press/release. Large
transcripts therefore take time to type. Keep the intended application focused
and avoid typing during delivery. Keyboard events cannot acknowledge inserted
text, pin focus universally, detect a recipient's pending compose state, or
prove that an application consumed every event.

## Validation

Automated tests capture events without creating a kernel device and replay
them into an independent XKB/compose receiver. They check literal strings on
US, US international, and German maps, long output, whole-burst rejection,
modifier changes, and failures at every event boundary. Output-chain tests
check that clipboard fallback is impossible. Existing tests that create kernel
input devices are ignored by default and must be selected explicitly in an
isolated input session.

Before enabling this driver for daily use, validate it in an isolated desktop
or VM with the actual compositor, keyd/X-keys configuration, terminal, and
editor. Hotplug readiness, application delivery, focus changes, remapper
exclusion, and console behavior need that integration evidence. The unit tests
do not establish those properties.
