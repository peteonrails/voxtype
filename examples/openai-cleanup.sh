#!/bin/bash
# Example post-processing script for Voxtype using OpenAI API
#
# Usage: Configure in ~/.config/voxtype/config.toml:
#   [output.post_process]
#   command = "/path/to/openai-cleanup.sh"
#   timeout_ms = 10000
#
# Requirements:
# - OPENAI_API_KEY environment variable set
# - curl and jq installed
#
# Tips:
# - gpt-4o-mini is fast and cheap for this use case
# - The prompt explicitly says "no emojis" because ydotool can't type them

set -euo pipefail

if [[ -z "${OPENAI_API_KEY:-}" ]]; then
    echo "Error: OPENAI_API_KEY not set" >&2
    cat  # Pass through original text on error
    exit 0
fi

INPUT=$(cat)

# Empty input = empty output
if [[ -z "$INPUT" ]]; then
    exit 0
fi

# Build prompt with optional context from previous dictation
SYSTEM_PROMPT="You clean up dictated text. Remove filler words (um, uh, like), fix grammar and punctuation. Output ONLY the cleaned text - no quotes, no emojis, no explanations."

if [[ -n "${VOXTYPE_CONTEXT:-}" ]]; then
  SYSTEM_PROMPT="${SYSTEM_PROMPT} You will receive the previous dictation for context - do NOT include it in your output, only clean up the current text."
fi

# Build JSON payload with jq to handle special characters
if [[ -n "${VOXTYPE_CONTEXT:-}" ]]; then
  JSON=$(jq -n --arg text "$INPUT" --arg system "$SYSTEM_PROMPT" --arg context "$VOXTYPE_CONTEXT" '{
    model: "gpt-4o-mini",
    messages: [
      { role: "system", content: $system },
      { role: "user", content: ("Previous dictation for context:\n" + $context + "\n\nCurrent text to clean up:\n" + $text) }
    ],
    max_tokens: 1000
  }')
else
  JSON=$(jq -n --arg text "$INPUT" --arg system "$SYSTEM_PROMPT" '{
    model: "gpt-4o-mini",
    messages: [
      { role: "system", content: $system },
      { role: "user", content: $text }
    ],
    max_tokens: 1000
  }')
fi

# Call OpenAI API
RESPONSE=$(curl -s --max-time 8 \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $OPENAI_API_KEY" \
    -d "$JSON" \
    https://api.openai.com/v1/chat/completions)

# Extract the response text
OUTPUT=$(echo "$RESPONSE" | jq -r '.choices[0].message.content // empty')

if [[ -n "$OUTPUT" ]]; then
    echo "$OUTPUT"
else
    # On error, pass through original text
    echo "$INPUT"
fi
