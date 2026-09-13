import os
import wave
import numpy as np
import subprocess

OUT_DIR = os.path.join(os.path.dirname(__file__), "test_clips")
os.makedirs(OUT_DIR, exist_ok=True)

CLIPS = {
    "clip1_5s.wav": "Let me know whether there are some changes that you would like to make.",
    "clip2_30s.wav": (
        "We are conducting a comprehensive evaluation of the speech recognition engine to determine transcription accuracy "
        "across different speech models and acoustic conditions. The quick brown fox jumps over the lazy dog. "
        "Please confirm that all parameters are functioning properly and that no words are being dropped at chunk seams. "
        "Local inference requires consistent acoustic processing, zero-padded frames, and robust decoding algorithms."
    ),
    "clip3_75s.wav": (
        "First item: check the audio pipeline and ensure sixteen kilohertz sample rate with mono float values. "
        "Second item: verify that the encoder cache states are properly preserved and carried between consecutive chunks. "
        "Third item: make sure language ID prompt tokens are properly provided for all multilingual speech models. "
        "Fourth item: check the input dynamic range and avoid overly aggressive soft-knee limiting or audio clipping. "
        "Fifth item: ensure windowing and fast Fourier transform parameters match the model preprocessor configuration exactly. "
        "Sixth item: verify that the token vocabulary correctly handles subwords, word pieces, and special language tags. "
        "Seventh item: validate that the hallucination guard rejects diverged outputs and restores deterministic transcripts. "
        "Eighth item: test long audio recordings to ensure that tail truncation never silently drops the final clauses. "
        "Ninth item: review the accuracy and word error rate across all local and cloud speech engines before deployment. "
        "Tenth item: confirm that streaming inference maintains low latency and stable memory usage throughout the dictation. "
        "Let me know whether there are some changes that you would like to make."
    )
}

for name, text in CLIPS.items():
    path = os.path.join(OUT_DIR, name)
    # Generate via SAPI PowerShell
    cmd = [
        "powershell", "-NoProfile", "-Command",
        f"Add-Type -AssemblyName System.Speech; "
        f"$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer; "
        f"$synth.SetOutputToWaveFile('{path}', (New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono))); "
        f"$synth.Speak('{text}'); "
        f"$synth.Dispose();"
    ]
    subprocess.run(cmd, check=True)
    with wave.open(path, 'rb') as wf:
        dur = wf.getnframes() / wf.getframerate()
        print(f"Generated {name}: {dur:.1f}s")
