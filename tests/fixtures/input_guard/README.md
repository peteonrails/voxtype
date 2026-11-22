# Dead-input guard fixtures

Two-second excerpts of real 16 kHz mono captures, used by
`tests/input_guard_integration.rs`. None of them carries speech: the live
ones are the quietest recordings of the corpus the guard's threshold was
measured on, which is what makes them the useful test. Peak frame levels
are measured with the guard's own statistic (150 Hz high-pass, 100 ms
frames, peak over 1.5 s after a 100 ms settle).

| file | source | peak frame | what it is |
|---|---|---|---|
| `dead_silence.wav` | 2026-11-04 capture, 20.6 s, no transcript | -inf dBFS | bit-exact silence: the failure this guard exists for |
| `dead_dc_drift.wav` | 2026-03-23 capture, 45.9 s, no transcript | -70.4 dBFS | DC offset plus sub-audible drift, no sound; the population the guard cannot separate from live audio |
| `live_quietest.wav` | 2026-04-08 capture | -82.8 dBFS | the quietest live capture of 22,811 |
| `live_quiet_recent.wav` | 2026-11-09 capture | -79.1 dBFS | quiet room, external microphone |
| `live_builtin_mic.wav` | laptop built-in microphone, quiet room | -50.8 dBFS | live microphone with a -0.21 full-scale hardware DC bias |
