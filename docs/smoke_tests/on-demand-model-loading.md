# On-Demand Model Loading

Tests loading model only when needed (reduces idle memory).

The setting belongs to the active engine: `whisper.on_demand_loading`,
`cohere.on_demand_loading`, `parakeet.on_demand_loading` and so on.

```bash
# 1. Enable on_demand_loading for the engine in use:
voxtype config get engine
voxtype config set whisper.on_demand_loading true    # or cohere., parakeet., ...
#    or by hand in config.toml:
#    [whisper]
#    on_demand_loading = true

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Check memory before recording (model not loaded):
systemctl --user status voxtype | grep Memory
#    A GGUF engine (Cohere, Parakeet) runs its model in a worker process,
#    which must not exist yet. This prints nothing:
pgrep -af 'voxtype-gguf-[w]orker'

# 4. Record and transcribe
voxtype record start && sleep 3 && voxtype record stop

# 5. Check logs for the model load:
journalctl --user -u voxtype --since "1 minute ago" | grep -E "On-demand loading enabled|Loading whisper model|GGUF worker ready|Model loaded"
# If that prints nothing: journalctl _SYSTEMD_USER_UNIT=voxtype.service --since "1 minute ago" | grep -E "On-demand loading enabled|Loading whisper model|GGUF worker ready|Model loaded"

# 6. Check the model is gone again, a few seconds after the text is typed:
systemctl --user status voxtype | grep Memory    # back near the step 3 figure
pgrep -af 'voxtype-gguf-[w]orker'                # GGUF engines: prints nothing
```

Expected log lines:

- At startup: `On-demand loading enabled, model will be loaded when recording starts`,
  and no `Loading transcription model:` line (that one is the startup preload
  with on-demand loading off).
- When recording starts, Whisper logs `Loading whisper model from ...` and
  `Model loaded in N.NNs`; a GGUF engine logs `Cohere GGUF worker ready in N.NNs`
  (or `Parakeet`).
- When recording stops: `Model loaded successfully`.

Nothing is logged when the model is unloaded, so step 6 checks the unload
itself: the daemon's memory falls back, and for a GGUF engine the worker
process is gone. With on-demand loading off the worker is there from startup
and stays.
