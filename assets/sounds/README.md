# Sounds

## meeting-announcement-kokoro.wav

The built-in recording announcement, played when a meeting starts with
`[meeting] announce_recording = true` (see docs/MEETING_MODE.md, "Announcing
the Recording"). Every build embeds it.

- Says: "This meeting is being recorded and transcribed with Voxtype." The
  text lives in `ANNOUNCEMENT_TEXT` in `src/meeting/announce.rs`.
- This is AI-generated speech, rendered locally with Kokoro-82M
  (hexgrad/Kokoro-82M, revision `f3ff3571791e39611d31c381e3a41a3af07b4987`),
  voice `af_heart`, speed 1.0.
- Format: 24 kHz mono 16-bit PCM WAV, 4.1 s, 195,644 bytes.
- sha256 of the file as rendered, committed unmodified:
  `13a25abe7dea3bbceaa45d83ab2a12b776879480d9607c5e424ecd65f35bb84f`

Kokoro-82M's weights and voices are released under the Apache License 2.0
(the model card's `license: apache-2.0`,
https://huggingface.co/hexgrad/Kokoro-82M). The model card places no terms
on the audio the model generates. This clip is covered by voxtype's MIT
licence, like everything else in this repository (`LICENSE`).

To render it again after changing the text, run
`scripts/render-meeting-announcement.sh --kokoro` against a local Kokoro
server with an OpenAI-compatible `/v1/audio/speech` endpoint (`KOKORO_URL`,
default `http://127.0.0.1:8842`), then update the size and sha256 above. A
re-render sounds the same but is not byte-identical, so the sha256 always
changes. Voxtype itself never calls a speech service: the clip ships in the
binary and plays offline.

## The Jessica voice (not in this repository)

`[meeting] announcement_voice = "jessica"` announces in an ElevenLabs voice
instead. That clip is not part of this repository, the source archive or any
package. Voxtype downloads it on request (`voxtype meeting announcement
--voice jessica`, or the first meeting that wants it) from
`https://models.voxtype.io/sounds/meeting-announcement-jessica-v1.wav`,
checks its sha256 against the one pinned in `src/meeting/announce/jessica.rs`
(`40267063245522143aeca0793f5a435e583beade0194efb041bfb2513cea7251`), and
keeps it in `~/.local/share/voxtype/sounds/`, with its terms beside it.

- AI-generated speech, rendered with ElevenLabs text-to-speech (voice:
  Jessica, model eleven_multilingual_v2), 24 kHz mono 16-bit WAV, 3.2 s,
  153,852 bytes, the API's raw PCM in a plain WAV header.
- Its terms are not MIT: it may be redistributed only unmodified, as part of
  voxtype or a package of voxtype, and it may not be used to train or improve
  AI models. Don't strip provenance metadata or watermarks from it.
- `scripts/render-meeting-announcement.sh` without `--kokoro` renders it again
  with an ElevenLabs API key, to a file outside this repository. A new
  rendering gets a new versioned object name on the CDN and a new pinned
  sha256.
