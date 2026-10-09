"""Compare decoder settings on the same prepared WAVs; no uploads or app writes.

Use replay_review --prepare-only first to apply the exact production audio chain.
JSON results contain transcripts, so save them outside the repository.
"""
import argparse
import gc
import json
import os
from pathlib import Path
import struct
import time
import wave

import numpy as np
import sherpa_onnx


def raw_diagnostics(path):
    data = Path(path).read_bytes()
    if data[:4] != b'RIFF' or data[8:12] != b'WAVE':
        raise ValueError('Expected a RIFF WAV')
    chunks = {}
    offset = 12
    while offset + 8 <= len(data):
        name, size = struct.unpack_from('<4sI', data, offset)
        chunks[name] = data[offset + 8:offset + 8 + size]
        offset += 8 + size + size % 2
    encoding, channels, rate, _, _, bits = struct.unpack_from('<HHIIHH', chunks[b'fmt '])
    if encoding == 0xfffe:  # WAVE_FORMAT_EXTENSIBLE: subtype GUID starts at byte 24.
        encoding = struct.unpack_from('<I', chunks[b'fmt '], 24)[0]
    if encoding != 3 or bits != 32:
        raise ValueError('Raw diagnostics expect the retained float32 WAV')
    samples = np.frombuffer(chunks[b'data'], '<f4').reshape(-1, channels)
    return {'file': Path(path).name, 'seconds': len(samples) / rate,
            'channel_rms': np.sqrt(np.mean(samples.astype(np.float64) ** 2, axis=0)).tolist(),
            'channel_peak': np.max(np.abs(samples), axis=0).tolist(),
            'clipped_samples': np.sum(np.abs(samples) >= .999, axis=0).tolist(),
            'stereo_correlation': float(np.corrcoef(samples[:, 0], samples[:, 1])[0, 1]) if channels == 2 else None,
            'mix_rms': float(np.sqrt(np.mean(np.mean(samples, axis=1).astype(np.float64) ** 2)))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('clips', nargs='*', type=Path)
    parser.add_argument('--raw-diagnostics', type=Path, nargs='+')
    parser.add_argument('--decoders', nargs='+', default=['modified_beam_search', 'greedy_search'],
                        choices=['modified_beam_search', 'greedy_search'])
    parser.add_argument('--scales', type=float, nargs='+', default=[1.0])
    parser.add_argument('--threads', type=int, default=6)
    args = parser.parse_args()
    if args.raw_diagnostics:
        for path in args.raw_diagnostics:
            print(json.dumps({'audio': raw_diagnostics(path)}), flush=True)
    if not args.clips:
        return
    clips = []
    for path in args.clips:
        with wave.open(str(path), 'rb') as wav:
            if wav.getnchannels() != 1 or wav.getsampwidth() != 2 or wav.getframerate() != 16000:
                raise ValueError('Expected prepared mono s16/16 kHz WAV')
            samples = np.frombuffer(wav.readframes(wav.getnframes()), '<i2').astype(np.float32) / 32768
        clips.append((path, samples))
    folder = Path(os.environ['APPDATA']) / 'com.typr.app/parakeet-tdt-0.6b-v3-int8'
    files = {key: str(folder / name) for key, name in [('encoder', 'encoder.int8.onnx'),
             ('decoder', 'decoder.int8.onnx'), ('joiner', 'joiner.int8.onnx'), ('tokens', 'tokens.txt')]}
    for decoder in args.decoders:
        recognizer = sherpa_onnx.OfflineRecognizer.from_transducer(**files,
            num_threads=args.threads, decoding_method=decoder, max_active_paths=8,
            model_type='nemo_transducer')
        for path, samples in clips:
            for scale in args.scales:
                started = time.perf_counter()
                stream = recognizer.create_stream()
                stream.accept_waveform(16000, np.clip(samples * scale, -1, 1))
                recognizer.decode_stream(stream)
                result = stream.result
                print(json.dumps({'file': path.name, 'decoder': decoder, 'scale': scale,
                    'decode_ms': round((time.perf_counter() - started) * 1000), 'text': result.text,
                    'timestamps': list(result.timestamps)}), flush=True)
                del stream
        del recognizer
        gc.collect()


if __name__ == '__main__':
    main()
