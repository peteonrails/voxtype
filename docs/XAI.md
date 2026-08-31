# xAI Grok Speech-to-Text

Batch STT after push-to-talk: `POST https://api.x.ai/v1/stt`. Audio leaves the machine. Not live captions.

```toml
engine = "xai"

[xai]
api_key = "xai-..."
# language = "en"   # omit or "auto" to autodetect (then format is omitted)
```

Or `VOXTYPE_XAI_API_KEY` / `XAI_API_KEY` / `--xai-api-key`. Then `voxtype config set engine xai` and restart the daemon.

xAI rejects `format=true` without a language code, so autodetect does not send `format`.
