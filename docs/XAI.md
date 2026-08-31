# xAI Grok Speech-to-Text

Batch STT after push-to-talk: `POST https://api.x.ai/v1/stt`. Audio leaves the machine. Not live captions.

## API key (console token billing)

```toml
engine = "xai"

[xai]
api_key = "xai-..."
# language = "en"   # omit or "auto" to autodetect (then format is omitted)
```

Or `VOXTYPE_XAI_API_KEY` / `XAI_API_KEY` / `--xai-api-key`.

## SuperGrok / X Premium+ (subscription quota)

Same STT quality, charged against the Grok or X plan you already use on grok.com / grok CLI — not console API tokens.

```bash
voxtype setup xai --login
voxtype config set engine xai
```

Tokens: `$XDG_DATA_HOME/voxtype/xai-oauth.json` (mode 0600). Login uses grok-cli's public OAuth client id (`b1a00492-073a-47ea-816f-4c329264a828`); xAI does not offer third-party CLI registration. The consent screen may say grok-cli. `--login --no-browser` prints the URL only. `voxtype setup xai --status` / `--logout`.

xAI rejects `format=true` without a language code, so autodetect does not send `format`.
