# Voice activity model: source and licence

The Windows installer ships `silero_vad_v5.onnx` under `<install>/vad/`, and the
engine downloads the same file (pinned by SHA-256) into the per-user cache when
no installed copy exists. The file itself is not kept in this repository: the
installer build fetches it from the address below and checks it against the pin.

## silero_vad_v5.onnx

- What it is: the Silero VAD voice activity detector, release v5.1.2, in ONNX
  form (16 kHz, 512-sample windows).
- Origin: `src/silero_vad/data/silero_vad.onnx` in the snakers4/silero-vad
  repository at commit `6478567951ae5c9979ad7b234185b5515f4be7a1` (the commit
  the `v5.1.2` tag names):
  https://raw.githubusercontent.com/snakers4/silero-vad/6478567951ae5c9979ad7b234185b5515f4be7a1/src/silero_vad/data/silero_vad.onnx
- Licence: MIT, Copyright (c) 2020-present Silero Team (the repository's
  `LICENSE` file).
- Changes: none by Handvel. The file is stored under the name
  `silero_vad_v5.onnx`, the name screenpipe uses for it.
- SHA-256: `2623a2953f6ff3d2c1e61740c6cdb7168133479b267dfef114a4a3cc5bdd788f`
  (2,327,524 bytes).

## MIT licence text

Copyright (c) 2020-present Silero Team

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
