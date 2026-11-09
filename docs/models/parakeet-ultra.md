# Parakeet Ultra model provenance

Ultra is one post-trained checkpoint of NVIDIA Parakeet TDT 0.6B v3, published
by Moondream. F16, Q8_0 and Q4_K_M are precision variants, not separate engines.
Moondream's model revision is `73175eb7aeb0d82f1e2a6b53b3aabc10a90bcd0b`:
https://huggingface.co/moondream/parakeet-ultra/tree/73175eb7aeb0d82f1e2a6b53b3aabc10a90bcd0b

GGUF conversion is by handy-computer at revision
`a1539be1b849ea7463b88343e5277b373d0c4ea9`:
https://huggingface.co/handy-computer/parakeet-ultra-gguf/tree/a1539be1b849ea7463b88343e5277b373d0c4ea9

The conversion's NOTICE says handy-computer repackaged, renamed and rearranged
tensors for transcribe.cpp; the Q variants additionally reduce numeric precision.
The converter did not retrain or fine-tune the weights. Both model cards declare
CC-BY-4.0. Retain attribution to NVIDIA, Moondream and handy-computer, a link to
the license, and the conversion notice when redistributing. This provenance
record is not a legal opinion; the model's terms are distinct from Voxtype's.

Pinned license and conversion notice:
- https://huggingface.co/handy-computer/parakeet-ultra-gguf/blob/a1539be1b849ea7463b88343e5277b373d0c4ea9/LICENSE
- https://huggingface.co/handy-computer/parakeet-ultra-gguf/blob/a1539be1b849ea7463b88343e5277b373d0c4ea9/NOTICE

| File | Bytes | SHA-256 |
|---|---:|---|
| `parakeet-ultra-F16.gguf` | 1256724448 | `cd97f94fa50a034a9019a47e2ed7a2de3963eed8316d1df4de9efdccca6fe78d` |
| `parakeet-ultra-Q8_0.gguf` | 740363168 | `007a59761e9258779f189df396b50d12b83bb5b79e66df4b955c230d2f2a0a59` |
| `parakeet-ultra-Q4_K_M.gguf` | 486280096 | `c69b7a5f9071a7afd8a1818ef7ae542c386c4b01ed0cac380c19a99947646bf9` |

Downloads use the revision-pinned conversion repository directly, an explicit
exception to the usual R2-only policy. There is no approved Ultra R2 mirror and
none is invented here. The downloader checks bytes and SHA-256 before promotion;
other models retain their original mirror URLs. No weights are bundled with the
application. Model publication needs separate approval.

The cards list 25 European languages: English, German, French, Spanish, Italian,
Portuguese, Russian, Ukrainian, Croatian, Slovenian, Latvian, Lithuanian,
Estonian, Finnish, Swedish, Danish, Dutch, Polish, Czech, Slovak, Hungarian,
Romanian, Bulgarian, Greek and Maltese. This runtime uses automatic language
recognition without selectable language hints. Translation, streaming and the
publisher's broader audio-segmentation pipeline are not provided by this addition.
Publisher benchmark claims are not Voxtype measurements.
