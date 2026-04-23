#!/bin/bash
# Example post-processing script for Voxtype using Google Gemini API
#
# Usage: Configure in ~/.config/voxtype/config.toml:
#   [output.post_process]
#   command = "/path/to/gemini-cleanup.sh"
#   timeout_ms = 10000
#
# Requirements:
# - GEMINI_API_KEY environment variable set (get one at https://aistudio.google.com/apikey)
# - curl and jq installed
#
# Tips:
# - gemini-2.0-flash is fast and cost-effective
# - The prompt explicitly says "no emojis" because ydotool can't type them

set -euo pipefail

if [[ -z "${GEMINI_API_KEY:-}" ]]; then
    echo "Error: GEMINI_API_KEY not set" >&2
    cat  # Pass through original text on error
    exit 0
fi

INPUT=$(cat)

# Empty input = empty output
if [[ -z "$INPUT" ]]; then
    exit 0
fi

# Build prompt with optional context from previous dictation
PROMPT="Clean up this dictated text. Remove filler words (um, uh, like), fix grammar and punctuation. Output ONLY the cleaned text - no quotes, no emojis, no explanations:"

if [[ -n "${VOXTYPE_CONTEXT:-}" ]]; then
  printf -v PROMPT '%s\n\nPrevious dictation for context (do NOT include this in your output):\n%s\n\nCurrent text to clean up:' "$PROMPT" "$VOXTYPE_CONTEXT"
fi

# Build JSON payload with jq to handle special characters
JSON=$(jq -n --arg text "$INPUT" --arg prompt "$PROMPT" '{
  contents: [
    {
      parts: [
        {
          text: ($prompt + "\n\n" + $text)
        }
      ]
    }
  ],
  generationConfig: {
    maxOutputTokens: 1000
  }
}')

# Call Gemini API
RESPONSE=$(curl -s --max-time 8 \
    -H "Content-Type: application/json" \
    -d "$JSON" \
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent?key=$GEMINI_API_KEY")

# Extract the response text
OUTPUT=$(echo "$RESPONSE" | jq -r '.candidates[0].content.parts[0].text // empty')

if [[ -n "$OUTPUT" ]]; then
    echo "$OUTPUT"
else
    # On error, pass through original text
    echo "$INPUT"
fi
