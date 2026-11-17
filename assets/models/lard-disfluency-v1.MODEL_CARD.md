# LARD disfluency tagger v1

`lard-disfluency-v1.vxdf` is a compact English token classifier for Voxtype's
trained disfluency stage. It labels token spans as fluent or disfluent. It does
not contain source utterances, a vocabulary, or recoverable rows from the
training data.

## Provenance and attribution

The model was trained from `train.csv` and thresholded on `validation.csv` in
LARD 1.0, DOI [10.5281/zenodo.6451984](https://doi.org/10.5281/zenodo.6451984):

- Tatiana Passali
- Thanassis Mavropoulos
- Grigorios Tsoumakas
- Georgios Meditskos
- Stefanos Vrochidis

LARD is derived from the Schema-Guided Dialogue (SGD) dataset by Abhinav
Rastogi, Xiaoxue Zang, Srinivas Sunkara, Raghav Gupta, and Pranav Khaitan.
SGD's project page and license are at
<https://github.com/google-research-datasets/dstc8-schema-guided-dialogue>.

The Zenodo record metadata identifies LARD as CC BY 4.0, while the README
inside the same deposit says CC BY-SA 4.0. SGD is also CC BY-SA 4.0. Voxtype
therefore distributes this derived model under the more restrictive
[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) terms. This
license applies to the `.vxdf` model artifact, not to Voxtype's MIT-licensed
source code.

Please cite:

> Tatiana Passali, Thanassis Mavropoulos, Grigorios Tsoumakas, Georgios
> Meditskos, and Stefanos Vrochidis. 2022. LARD: Large-scale Artificial
> Disfluency Generation. Proceedings of LREC 2022, pages 2327-2336.

## Reproduction

No LARD CSV file is committed or packaged. Download the three published files
directly from the Zenodo record, then run:

```console
python3 scripts/train-lard-disfluency.py \
  --train /path/to/train.csv \
  --validation /path/to/validation.csv \
  --output assets/models/lard-disfluency-v1.vxdf
```

The trainer uses only the Python standard library. It verifies the deposited
MD5 identities before reading either split. Training is deterministic: three
epochs, 32,768 hashed features, seed 696, weighted logistic updates, int16
quantization, and a high-precision hysteresis threshold selected on the
validation split.

Generated with Python 3.12.14 on 2026-11-07:

- Validation token precision: 0.9519
- Validation token recall: 0.3691
- Validation token F1: 0.5319
- Anchor/continuation thresholds: 3.9 / 2.0
- Artifact size: 65,560 bytes
- SHA-256: `f11b45d644b2d829349df2ac2ad3d69c9cf19cd3fae888c69268bddf64cfb354`

The operating point intentionally favors precision. This model will leave
some disfluencies in place rather than make an aggressive deletion from a
user's dictation.

## Published input identities

| File | MD5 | Use |
|---|---|---|
| `train.csv` | `ada1e11d565c04779940100cb9323ba8` | Weight training |
| `validation.csv` | `af36afcd23712d3ebc01064a5e65ede0` | Threshold selection |
| `test.csv` | `ccba0fd9f1e3fb973961527be6c58762` | Reserved held-out split |

LARD's raw CSV files are neither copied nor transformed into repository
fixtures. The deterministic Rust regression fixtures are short, original
sentences created for Voxtype and carry the repository's MIT license.
