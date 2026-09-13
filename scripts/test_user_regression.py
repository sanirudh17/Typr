import os, subprocess, sherpa_onnx, wave, numpy as np

APPDATA = os.path.expandvars(r'%APPDATA%\com.typr.app')
NEMOTRON_DIR = os.path.join(APPDATA, 'nemotron-3.5-asr-streaming-0.6b-int8')
wav_path = os.path.join(APPDATA, 'test_user_regression.wav')

text = 'The thing is, the Nemotron model accuracy is so bad. The problem is that I dictate something perfect, and the cloud and Parakeet models are able to capture perfectly, but the Nemotron model completely misunderstands everything despite having the AI post processing on. I want to take an investigative approach for this and investigate this deeply, please fix.'

cmd = [
    'powershell', '-NoProfile', '-Command',
    f'Add-Type -AssemblyName System.Speech; '
    f'$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer; '
    f'$synth.SetOutputToWaveFile("{wav_path}", (New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono))); '
    f'$synth.Speak("{text}"); '
    f'$synth.Dispose();'
]
subprocess.run(cmd, check=True)

with wave.open(wav_path, 'rb') as wf:
    sr = wf.getframerate()
    samples = np.frombuffer(wf.readframes(wf.getnframes()), dtype=np.int16).astype(np.float32) / 32768.0

recognizer = sherpa_onnx.OnlineRecognizer.from_transducer(
    encoder=os.path.join(NEMOTRON_DIR, 'encoder.int8.onnx'),
    decoder=os.path.join(NEMOTRON_DIR, 'decoder.int8.onnx'),
    joiner=os.path.join(NEMOTRON_DIR, 'joiner.int8.onnx'),
    tokens=os.path.join(NEMOTRON_DIR, 'tokens.txt'),
    num_threads=2,
    decoding_method='greedy_search'
)

for lang in ['en', 'auto', '']:
    s = recognizer.create_stream()
    if lang:
        s.set_option('language', lang)
    s.accept_waveform(sr, samples)
    s.input_finished()
    while recognizer.is_ready(s):
        recognizer.decode_stream(s)
    print(f"Lang [{lang:4}]: {recognizer.get_result(s)}")
