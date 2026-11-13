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

## Licence

`meeting-announcement.wav` is not covered by voxtype's MIT licence. It may
be redistributed, unmodified, as part of voxtype and of packages of voxtype.
It may not be used to train or improve AI models. Everything else in this
repository stays under the MIT licence in `LICENSE`.
