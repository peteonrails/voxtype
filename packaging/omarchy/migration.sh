echo "Add push-to-talk voice-to-text (voxtype)"

# Install voxtype and wtype (for keyboard simulation)
omarchy-pkg-add voxtype wtype

# Enable state file for Waybar integration
VOXTYPE_CFG="$HOME/.config/voxtype/config.toml"
mkdir -p "$(dirname "$VOXTYPE_CFG")"

if [ ! -f "$VOXTYPE_CFG" ]; then
  # Create config with state_file enabled
  cat > "$VOXTYPE_CFG" << 'EOF'
# Voxtype Configuration

[hotkey]
key = "SCROLLLOCK"
modifiers = []

[whisper]
model = "base.en"
language = "en"

[output]
mode = "type"
fallback_to_clipboard = true

[output.notification]
on_transcription = true

# Enable Waybar integration
state_file = "auto"
EOF
else
  # Add state_file if not already present
  if ! grep -q "state_file" "$VOXTYPE_CFG"; then
    echo "" >> "$VOXTYPE_CFG"
    echo "# Enable Waybar integration" >> "$VOXTYPE_CFG"
    echo 'state_file = "auto"' >> "$VOXTYPE_CFG"
  fi
fi

# Add Waybar module ONLY if not already present
WAYBAR_CFG="$HOME/.config/waybar/config.jsonc"

if [ -f "$WAYBAR_CFG" ]; then
  if ! grep -q "custom/voxtype" "$WAYBAR_CFG"; then
    echo "Patching Waybar config to add voxtype module"

    # Add voxtype module definition before the closing brace
    # This is a simplified approach - users may need to manually add to modules-right
    if grep -q '"modules-right"' "$WAYBAR_CFG"; then
      # Try to add "custom/voxtype" to modules-right array
      sed -i 's/"modules-right": \[/"modules-right": ["custom\/voxtype", /' "$WAYBAR_CFG"
    fi
  fi
fi

# Enable and start the systemd user service
systemctl --user enable voxtype
systemctl --user start voxtype

# Reload Waybar
omarchy-restart-waybar

echo ""
echo "Voxtype installed! Hold SCROLLLOCK to record, release to transcribe."
echo "Run 'voxtype setup --download' if you need to download a whisper model."
