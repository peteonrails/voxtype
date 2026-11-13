# Whistle vs. Whisper: initial local comparison

This is a reproducible smoke benchmark, not a representative accuracy evaluation. The
clips use eSpeak NG's synthetic French voice because this checkout has no recorded
French/technical speech fixtures. Results should not be used to change Voxtype's
engine defaults. Repeat with consenting human speakers and production microphones
before drawing conclusions about real dictation.

## Environment and method

Run on 2026-10-04 on an Intel Core i5-4440 (4 cores, 3.10 GHz), using the same
Voxtype build and four identical 16 kHz mono WAV clips for both engines. The clips
were generated with eSpeak NG 1.52.0.1 (`-v fr-fr -s 145`) from these sentences:

1. `Je travaille sur SvelteKit avec PostgreSQL et GitLab.`
2. `Ouvre le projet Voxtype sur NixOS.`
3. `Je vais créer une branche pour corriger le composant Skeleton.`
4. `Demain à quinze heures trente, j'ai un rendez-vous.`

Whistle used the `Cactus-Compute/whistle` `whistle.cact` model (16,919,407 bytes),
forced French, first with the seven named terms as keyword hints and then with no
hints. Whisper used the multilingual `ggml-base.bin` model (147,951,465 bytes),
forced French. Each timing is a fresh `voxtype transcribe` process wall time, including process
start, model initialization and inference; the OS page cache was not cleared.
Each clip was run once per engine/configuration. The median of four runs is
reported. The backend logs model initialization separately: Whistle reported
386 ms on its first run and 32–41 ms on subsequent warm-file-cache runs; Whisper
reported 2.21 s on its first run and 0.58–1.01 s on subsequent runs. Peak RSS
and CPU utilization were not measured.

Generate matching clips with:

```bash
texts=(
  'Je travaille sur SvelteKit avec PostgreSQL et GitLab.'
  'Ouvre le projet Voxtype sur NixOS.'
  'Je vais créer une branche pour corriger le composant Skeleton.'
  "Demain à quinze heures trente, j'ai un rendez-vous."
)
for i in "${!texts[@]}"; do
  espeak-ng -v fr-fr -s 145 -w "raw-$i.wav" "${texts[$i]}"
  ffmpeg -i "raw-$i.wav" -ar 16000 -ac 1 -sample_fmt s16 "clip-$i.wav"
done
```

Set up each engine in `config.toml`, keeping both models at local paths:

```toml
engine = "whistle"

[whistle]
model = "/path/to/whistle.cact"
language = "fr"
keywords = ["SvelteKit", "PostgreSQL", "GitLab", "Voxtype", "NixOS", "Hyprland", "Skeleton"]
runtime = "/path/to/libneedle.so"

[whisper]
model = "/path/to/ggml-base.bin"
language = "fr"
```

Run the same clips with `voxtype --config config.toml transcribe clip.wav --engine whistle`
and `--engine whisper`. For the no-keyword Whistle case, remove `keywords` from a copy
of the config. Measure cold wall time with a shell timer or `/usr/bin/time`.

## Results

| Engine/config | Cold wall times (s), clips 1–4 | Median (s) | Model size |
|---|---:|---:|---:|
| Whistle, keywords | 1.245, 0.835, 0.660, 0.475 | 0.748 | 16.1 MiB |
| Whistle, no keywords | 0.697, 0.478, 0.853, 0.666 | 0.682 | 16.1 MiB |
| Whisper base, French | 34.725, 19.751, 19.486, 25.644 | 22.698 | 141.1 MiB |

Transcripts from the keyword-enabled runs:

| Reference | Whistle, keywords | Whistle, no keywords | Whisper base |
|---|---|---|---|
| Je travaille sur SvelteKit avec PostgreSQL et GitLab. | Je travaille sur le smelt qui est un mec post-mrescule à la GitLab. | Je travaille sur le smelt qui est un mec post-mrescule à l'égide là. | Et j'ai pas maille du smite, qui t'a même pas de rescueil et rilab. |
| Ouvre le projet Voxtype sur NixOS. | Le problème est que ce n'est pas une épisodique. | Le problème, c'est que ce n'est pas une épisodique. | ou au-dessus de la moque Steve, sous-mique source. |
| Je vais créer une branche pour corriger le composant Skeleton. | Je mettrai une parce que vous pourriez être le temps pour masquer le temps. | Je mettrai une parce que vous pourriez être le troupeau masculin. | Je m'écris un mars bout de ta vie et je coupe un mars que tout. |
| Demain à quinze heures trente, j'ai un rendez-vous. | Demain à 15h30, j'ai expandé cette rendez-vous. | Demain à 15h30, j'ai expandé cette rendez-vous. | Le macameur de GX-7 et un ordénement. |

On this synthetic voice both models performed poorly. Whistle was much faster and
smaller in this run, but keyword hints did not measurably improve these four
transcripts. This is a wiring/performance smoke test only: it does not establish
real-world French or proper-name accuracy, and the single-sample timings are not a
statistical performance claim.

## Privacy check

The integration uses Needle's native shared library directly, not its Python package
or CLI. The upstream Python telemetry module documents opt-out environment
variables; Whistle sets `NEEDLE_TELEMETRY=0` and `DO_NOT_TRACK=1` before loading the
library. The tested Linux shared library exported the inference API and contained no
telemetry endpoint or telemetry-related strings. No telemetry behavior was observed
in the local transcription test.
