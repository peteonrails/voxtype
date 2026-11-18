# Use a local audio chat model for dictation

Some speech models serve transcription at `/v1/chat/completions` and accept
WAV data in an `audio_url` content part. This optional adapter lets Voxtype’s
existing Whisper remote backend use that interface. It converts each completed
recording to a single transcription request and returns the model’s text.
If the server already exposes a compatible `/v1/audio/transcriptions`
endpoint, point Voxtype at its base URL directly. Use this adapter for a
chat-only endpoint or to send Voxtype’s prompt and profile vocabulary to the
audio model as context.

1. Start a compatible speech model server on loopback. Follow the model card’s
   vLLM instructions, including hardware and version requirements. Examples
   with this wire format are [IBM Granite Speech 5.0 2B](https://huggingface.co/ibm-granite/granite-speech-5.0-2b)
   and [NVIDIA Nemotron 3.5 Transcribe 30B A4B](https://huggingface.co/nvidia/NVIDIA-Nemotron-3.5-Transcribe-30B-A4B).
2. Run `uv run examples/audio-chat-asr/adapter.py`. The adapter listens on
   `127.0.0.1:8011` and connects to `127.0.0.1:8000`. Use `--upstream-port`
   if the model server listens on a different port.
3. Try a saved WAV with a separate config before selecting a dictation model:

   ```toml
   engine = "whisper"
   [whisper]
   mode = "remote"
   remote_endpoint = "http://127.0.0.1:8011"
   remote_model = "ibm-granite/granite-speech-5.0-2b"
   remote_timeout_secs = 130
   translate = false
   ```

   `voxtype --config /path/to/test.toml transcribe /path/to/speech.wav`

Use the exact model ID served by your local server. An optional remote API key
is forwarded as a bearer token. Both ports should be accessible only to
trusted local clients. This example handles file transcription. The selected
model's input length and language limits apply. IBM documents up to about two
minutes for its hosted example. A truncated or malformed completion is
returned as an error so partial text is not submitted as a finished dictation.
