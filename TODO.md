# ML Diarization — In Progress

Branch: `feature/fix-ml-diarization`

Session notes: `~/Documents/markdown-notes/Voxtype/session-2026-04-12-ml-diarization-wiring.md`

## Resume here

1. Restart daemon with current build, start meeting, check similarity scores in logs
2. Tune similarity threshold if scores are low but consistent per-speaker
3. Verify ECAPA model input format (81MB model may expect preprocessed audio)
4. Consider TitaNet swap if embeddings are fundamentally noisy on short segments
