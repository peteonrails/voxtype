# Voxtype Hammerspoon Integration

Hammerspoon integration for voxtype on macOS. This is an alternative to the built-in hotkey support that doesn't require granting Accessibility permissions to Terminal.

## Installation

1. Install Hammerspoon:
   ```bash
   brew install --cask hammerspoon
   ```

2. Copy the voxtype module:
   ```bash
   cp voxtype.lua ~/.hammerspoon/
   ```

3. Add to your `~/.hammerspoon/init.lua`:
   ```lua
   local voxtype = require("voxtype")
   voxtype.setup({ hotkey = "rightalt" })
   ```

4. Reload Hammerspoon config (Cmd+Shift+R or click menu bar icon → Reload Config)

## Configuration

```lua
voxtype.setup({
    -- Key to use for push-to-talk
    -- Options: "rightalt", "rightcmd", "f13", "f14", etc.
    hotkey = "rightalt",

    -- Mode: "push_to_talk" or "toggle"
    -- push_to_talk: Hold key to record, release to transcribe
    -- toggle: Press once to start, press again to stop
    mode = "push_to_talk",

    -- Path to voxtype binary (optional, auto-detected)
    voxtype_path = nil,
})
```

## Adding a Cancel Hotkey

You can add a separate hotkey to cancel recording:

```lua
voxtype.add_cancel_hotkey({"cmd", "shift"}, "escape")
```

## Checking Status

```lua
print(voxtype.status())  -- Returns: "idle", "recording", "transcribing", or "stopped"
```

## Why Use Hammerspoon?

- **No Accessibility permissions for Terminal**: The built-in rdev hotkey requires granting Accessibility access to your terminal app
- **More flexible hotkey options**: Hammerspoon supports complex key combinations
- **Integration with other automations**: Combine voxtype with your other Hammerspoon workflows
- **Visual feedback**: Easy to add custom alerts and notifications

## Troubleshooting

**Hotkey not working?**
- Make sure Hammerspoon has Accessibility permissions (System Settings → Privacy & Security → Accessibility)
- Check the Hammerspoon console for errors (click menu bar icon → Console)
- Verify voxtype daemon is running: `voxtype status`

**voxtype not found?**
- Set the path explicitly: `voxtype.setup({ voxtype_path = "/path/to/voxtype" })`
- Or add voxtype to your PATH
