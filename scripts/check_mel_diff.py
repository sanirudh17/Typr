import numpy as np
import wave

with wave.open('scripts/test_clips/clip1_5s.wav', 'rb') as wf:
    sr = wf.getframerate()
    samples = np.frombuffer(wf.readframes(wf.getnframes()), dtype=np.int16).astype(np.float32) / 32768.0

n_fft = 512
hop_length = 160
win_length = 400
n_mels = 128

# Standard Hann window
window = 0.5 * (1.0 - np.cos(2.0 * np.pi * np.arange(win_length) / (win_length - 1)))

# Slaney/HTK mel scale
hz_to_mel = lambda hz: 2595.0 * np.log10(1.0 + hz / 700.0)
mel_to_hz = lambda mel: 700.0 * (10.0**(mel / 2595.0) - 1.0)
mel_min = hz_to_mel(0.0)
mel_max = hz_to_mel(8000.0)
mel_points = mel_to_hz(np.linspace(mel_min, mel_max, n_mels + 2))
bin_points = np.floor((n_fft + 1) * mel_points / sr).astype(int)

py_mels = []
for frame_start in range(0, len(samples) - win_length + 1, hop_length):
    frame = samples[frame_start:frame_start + win_length] * window
    fft_res = np.fft.rfft(frame, n_fft)
    power = np.abs(fft_res)**2
    
    frame_mel = np.zeros(n_mels, dtype=np.float32)
    for m in range(n_mels):
        s, c, e = bin_points[m], bin_points[m+1], bin_points[m+2]
        if c > s:
            frame_mel[m] += np.sum(((np.arange(s, c) - s) / (c - s)) * power[s:c])
        if e > c:
            frame_mel[m] += np.sum(((e - np.arange(c, e)) / (e - c)) * power[c:e])
    log_mel = np.log(frame_mel + 1e-10)
    py_mels.append(log_mel)

py_mels = np.array(py_mels)
print(f"Python log-mel shape: {py_mels.shape}")
print(f"Python stats: min={py_mels.min():.4f}, max={py_mels.max():.4f}, mean={py_mels.mean():.4f}")
