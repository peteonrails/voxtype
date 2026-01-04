#!/bin/bash
# Example post-processing script for Voxtype using Ollama
#
# Usage: Configure in ~/.config/voxtype/config.toml:
#   [output.post_process]
#   command = "/path/to/ollama-cleanup.sh"
#   timeout_ms = 30000
#
# Requirements:
# - Ollama running locally (ollama serve)
# - A model pulled (e.g., ollama pull llama3.2:1b)
#
# Tips:
# - Use small, fast models (1-3B) for lower latency
# - Use instruct/chat models, NOT reasoning models (they output <think> blocks)
# - The prompt explicitly says "no emojis" because ydotool can't type them

INPUT=$(cat)

# Build JSON payload properly with jq to handle special characters
JSON=$(jq -n --arg text "$INPUT" '{
  model: "llama3.2:1b",
  prompt: ("Clean up this dictated text. Remove filler words (um, uh, like), fix grammar and punctuation. Output ONLY the cleaned text - no quotes, no emojis, no explanations:\n\n" + $text),
  stream: false
}')

# Call Ollama API and extract response
# The sed commands: strip quotes, remove <think>...</think> blocks from reasoning models
OUTPUT=$(curl -s http://localhost:11434/api/generate -d "$JSON" \
  | jq -r '.response // empty' \
  | sed 's/^"//;s/"$//' \
  | sed 's/<think>.*<\/think>//g' \
  | tail -1)

echo "$OUTPUT"
