# Sounds

## meeting-announcement.wav

The built-in recording announcement, played when a meeting starts with
`[meeting] announce_recording = true` (see docs/MEETING_MODE.md, "Announcing
the Recording").

- Says: "This meeting is being recorded and transcribed with Voxtype." The
  text lives in `ANNOUNCEMENT_TEXT` in `src/meeting/announce.rs`.
- This is AI-generated speech. Announcement voice rendered with ElevenLabs
  text-to-speech (voice: Jessica, model eleven_multilingual_v2).
- Format: 24 kHz mono 16-bit PCM WAV, 3.2 s, 153,852 bytes.
- sha256 of the file as rendered, committed unmodified:
  `40267063245522143aeca0793f5a435e583beade0194efb041bfb2513cea7251`

The file is the API's raw PCM output (`output_format=pcm_24000`) wrapped in a
plain WAV header, with nothing removed or re-encoded. The PCM stream carries
no container metadata to preserve. If the clip is ever transcoded (to Opus,
say), carry any source tags across (`ffmpeg -map_metadata 0`) and do not strip
provenance metadata or watermarks.

To render it again after changing the text, run
`scripts/render-meeting-announcement.sh` with an ElevenLabs API key, then
update the size and sha256 above. Voxtype itself never calls ElevenLabs: the
clip ships in the binary and plays offline.

## meeting-announcement-kokoro.wav

The same announcement in an open-weight voice. Builds with the
`announcement-kokoro` cargo feature embed this clip instead of
`meeting-announcement.wav`, and only one of the two is compiled in. The Nix
flake turns the feature on for every package; other builds keep the clip
above.

- Says the same `ANNOUNCEMENT_TEXT`.
- This is AI-generated speech, rendered locally with Kokoro-82M
  (hexgrad/Kokoro-82M, revision `f3ff3571791e39611d31c381e3a41a3af07b4987`),
  voice `af_heart`, speed 1.0.
- Format: 24 kHz mono 16-bit PCM WAV, 4.1 s, 195,644 bytes.
- sha256 of the file as rendered, committed unmodified:
  `13a25abe7dea3bbceaa45d83ab2a12b776879480d9607c5e424ecd65f35bb84f`

Kokoro-82M's weights and voices are released under the Apache License 2.0
(the model card's `license: apache-2.0`,
https://huggingface.co/hexgrad/Kokoro-82M). The model card places no terms
on the audio the model generates.

To render it again after changing the text, run
`scripts/render-meeting-announcement.sh --kokoro` against a local Kokoro
server with an OpenAI-compatible `/v1/audio/speech` endpoint (`KOKORO_URL`,
default `http://127.0.0.1:8842`), then update the size and sha256 above. A
re-render sounds the same but is not byte-identical, so the sha256 always
changes.

## Licence

`meeting-announcement.wav` is not covered by voxtype's MIT licence. It may
be redistributed, unmodified, as part of voxtype and of packages of voxtype.
It may not be used to train or improve AI models.

`meeting-announcement-kokoro.wav` is covered by voxtype's MIT licence, like
everything else in this repository (`LICENSE`).
