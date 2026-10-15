# Speaker model weights: sources and licences

The Windows installer ships the two ONNX files in this folder under
`<install>/models/pyannote/`, and the engine downloads the same bytes (pinned by
SHA-256) when no installed copy exists. Freenpipe redistributes both files
unchanged, exactly as they were imported with the MIT base repository.

## segmentation-3.0.onnx

- What it is: the pyannote speaker-segmentation model, version 3.0, exported to
  ONNX.
- Origin: `pyannote/segmentation-3.0` on the Hugging Face hub, part of the
  pyannote.audio project (Hervé Bredin and contributors).
- Licence: MIT, as declared on the model card.
- Reference: Alexis Plaquet and Hervé Bredin, "Powerset multi-class cross
  entropy loss for neural speaker diarization", Proc. Interspeech 2023.
- SHA-256: `b78fc48113bb46fd247ae6a9aea737079550c647638db961df7e0e1e9f4ba62e`
  (5,983,836 bytes).

## wespeaker_en_voxceleb_CAM++.onnx

- What it is: a CAM++ speaker-embedding network trained on VoxCeleb, from the
  WeSpeaker toolkit (wenet-e2e/wespeaker), in the ONNX form with model metadata
  that the k2-fsa/sherpa-onnx project publishes for its speaker-recognition
  models.
- Licence: Creative Commons Attribution 4.0 International (CC BY 4.0,
  https://creativecommons.org/licenses/by/4.0/). WeSpeaker's pretrained-model
  documentation states that models trained on VoxCeleb follow the licence of
  the VoxCeleb dataset, which is CC BY 4.0. The WeSpeaker code is Apache-2.0.
- Credit: the WeSpeaker authors (Hongji Wang et al., "WeSpeaker: A Research and
  Production oriented Speaker Embedding Learning Toolkit", ICASSP 2023); the
  CAM++ architecture (Hui Wang et al., "CAM++: A Fast and Efficient Network for
  Speaker Verification Using Context-Aware Masking", Interspeech 2023); and the
  VoxCeleb datasets (Arsha Nagrani, Joon Son Chung, Andrew Zisserman, VGG,
  University of Oxford).
- Changes: none by Freenpipe.
- SHA-256: `c46fad10b5f81e1aa4a60c162714208577093655076c5450f8c469e522ec54ef`
  (29,292,684 bytes).
