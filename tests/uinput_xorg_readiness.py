#!/usr/bin/env python3
"""Check the readiness helper against a private Xorg server; emit no key events."""
import ctypes
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def run(args, env, text=None, check=True):
    return subprocess.run(args, env=env, input=text, text=True,
                          capture_output=True, check=check, timeout=10)


def printable_symbols(text):
    lib = ctypes.CDLL("libxkbcommon.so.0")
    for name, args, result in [
        ("xkb_context_new", [ctypes.c_int], ctypes.c_void_p),
        ("xkb_keymap_new_from_string", [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int, ctypes.c_int], ctypes.c_void_p),
        ("xkb_state_new", [ctypes.c_void_p], ctypes.c_void_p),
        ("xkb_state_update_key", [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int], ctypes.c_int),
        ("xkb_state_key_get_one_sym", [ctypes.c_void_p, ctypes.c_uint], ctypes.c_uint),
        ("xkb_keysym_to_utf32", [ctypes.c_uint], ctypes.c_uint),
        ("xkb_state_unref", [ctypes.c_void_p], None),
        ("xkb_keymap_unref", [ctypes.c_void_p], None),
        ("xkb_context_unref", [ctypes.c_void_p], None),
    ]:
        function = getattr(lib, name)
        function.argtypes, function.restype = args, result
    context = lib.xkb_context_new(0)
    keymap = lib.xkb_keymap_new_from_string(context, text.encode(), 1, 0)
    assert keymap, "Cannot decode receiver keymap"
    symbols = {}
    try:
        for modifiers in [(), (50,), (108,), (50, 108)]:
            state = lib.xkb_state_new(keymap)
            try:
                for key in modifiers:
                    lib.xkb_state_update_key(state, key, 1)
                for key in range(8, 256):
                    symbol = lib.xkb_state_key_get_one_sym(state, key)
                    character = lib.xkb_keysym_to_utf32(symbol)
                    if (character and chr(character).isprintable()) or 0xFE50 <= symbol <= 0xFE6F:
                        # Dead-key identity matters to compose processing even
                        # though the key itself has no Unicode character.
                        symbols[modifiers, key] = (character, symbol if not character else 0)
            finally:
                lib.xkb_state_unref(state)
    finally:
        lib.xkb_keymap_unref(keymap)
        lib.xkb_context_unref(context)
    return symbols


def main():
    read_fd, write_fd = os.pipe()
    server = subprocess.Popen(
        ["Xvfb", "-displayfd", str(write_fd), "-screen", "0", "640x480x24",
         "-nolisten", "tcp", "-noreset", "-ac"],
        pass_fds=(write_fd,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    os.close(write_fd)
    try:
        with os.fdopen(read_fd) as display_pipe:
            display = ":" + display_pipe.readline().strip()
        assert display != ":", "Private Xorg server did not start"
        env = dict(os.environ, DISPLAY=display)
        run(["xinput", "create-master", "Voxtype isolated"], env)
        name = "Voxtype isolated XTEST keyboard"
        device_id = run(["xinput", "list", "--id-only", name], env).stdout.strip()
        assert int(device_id) > 5
        core_before = run(["xkbcomp", "-w", "0", "-xkb", "-i", "3", display, "-"], env).stdout
        help_text = run(["xkbcli", "compile-keymap", "--help"], env).stdout
        # Match xkbcommon 0.8's KEYMAP_FORMAT_TEXT_V1, including on newer runtimes.
        format_args = ["--output-format", "1"] if "--output-format" in help_text else []
        for layout, variant in [("us", "intl"), ("de", "")]:
            native = run(["xkbcli", "compile-keymap", "--rules", "evdev", "--model", "pc105",
                          "--layout", layout, "--variant", variant, "--options", ""] + format_args, env).stdout
            original = run(["xkbcomp", "-w", "0", "-i", device_id, "-", display],
                           env, native, check=False)
            run([str(ROOT / "scripts/uinput-xorg-ready"), layout, variant],
                dict(env, VOXTYPE_UINPUT_DEVICE_NAME=name), native)
            actual = run(["xkbcomp", "-w", "0", "-xkb", "-i", device_id, display, "-"], env).stdout
            expected = printable_symbols(native)
            receiver = printable_symbols(actual)
            assert expected == receiver, "Xorg receiver changes printable key meanings"
            print(f"{layout}/{variant or 'base'}: helper accepted; {len(expected)} printable/dead-key strokes match; native upload exit {original.returncode}")
        core_after = run(["xkbcomp", "-w", "0", "-xkb", "-i", "3", display, "-"], env).stdout
        assert core_before == core_after, "Helper changed the core keyboard map"
        bad = run([str(ROOT / "scripts/uinput-xorg-ready"), "us", "intl"],
                  dict(env, VOXTYPE_UINPUT_DEVICE_NAME="Virtual core XTEST keyboard"), "map", check=False)
        assert bad.returncode != 0, "Helper accepted the core XTEST keyboard"
        print("Core keyboard unchanged; core-device rejection passed; no key events emitted")
    finally:
        server.terminate()
        server.wait(timeout=5)


if __name__ == "__main__":
    main()
