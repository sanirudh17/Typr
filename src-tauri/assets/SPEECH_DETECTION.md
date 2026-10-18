# Bundled speech detector

Typr uses Silero VAD locally to distinguish speech from silence and non-speech
noise before transcription. It does not remove or trim audio from a recording.
The detector runs on an independently normalized copy; no audio is uploaded by it.

- Upstream: https://github.com/snakers4/silero-vad
- Revision: `1e261b036686cd0017d500ee96acd1c4ba572a9d`
- File: `src/silero_vad/data/silero_vad.onnx`
- Bytes: 2,327,524
- SHA-256: `1a153a22f4509e292a94e67d6f9b85e8deb25b4988682b7e174c65279d8788e3`
- License: MIT, copyright Silero Team; see `SILERO_LICENSE.txt`, also included in
  packaged application resources.

The embedded bytes are materialized in the current user's local Typr cache for
the existing sherpa-onnx runtime. No model download or Python installation is
required. A detector load failure falls back to the preceding signal-level gate
and is logged, rather than blocking all dictation.
