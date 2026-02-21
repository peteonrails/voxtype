# SenseVoice Test Audio Files

Test WAV files for validating SenseVoice (and Sherpa) CJK transcription.

All files are 16-bit PCM, mono, 16kHz. Source:
[sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17](https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/tree/main/test_wavs)

## Files

| File | Language | Duration | Reference transcription |
|------|----------|----------|------------------------|
| zh.wav | Chinese (Mandarin) | 5.5s | 开放时间早上9点至下午5点。 |
| ja.wav | Japanese | 7.2s | (verify against model) |
| ko.wav | Korean | 4.6s | (verify against model) |
| yue.wav | Cantonese | 5.1s | (verify against model) |

The Chinese reference comes from the sherpa-onnx documentation. Japanese, Korean,
and Cantonese references are not documented upstream; run through the model and
compare against sherpa-onnx output to establish baselines.

## Usage

```bash
# Test with SenseVoice
voxtype transcribe tests/fixtures/sensevoice/zh.wav --engine sensevoice

# Compare against Whisper for the same file
voxtype transcribe tests/fixtures/sensevoice/zh.wav --engine whisper
```
