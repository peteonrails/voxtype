#!/bin/bash

# Waybar indicator for Voxtype recording state
# Returns JSON for waybar custom module

# Check if voxtype service is running
if ! systemctl --user is-active --quiet voxtype; then
    echo '{"text": ""}'
    exit 0
fi

# Read state from voxtype status file
STATE_FILE="$HOME/.local/state/voxtype/status"
if [ -f "$STATE_FILE" ]; then
    STATE=$(cat "$STATE_FILE" 2>/dev/null || echo "idle")
    case "$STATE" in
        recording)
            echo '{"text": "", "tooltip": "Recording...", "class": "recording"}'
            ;;
        transcribing)
            echo '{"text": "", "tooltip": "Transcribing...", "class": "transcribing"}'
            ;;
        *)
            echo '{"text": "", "tooltip": "Voxtype ready (hold HOME to record)", "class": "idle"}'
            ;;
    esac
else
    echo '{"text": "", "tooltip": "Voxtype ready", "class": "idle"}'
fi
