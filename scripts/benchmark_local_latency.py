"""Compare local CPU thread counts without changing decoder accuracy settings.

Uses only the existing reference WAVs, never microphone history or a cloud API.
The 20-second excerpt is a throughput probe, not the production chunking policy.
"""
import argparse
import gc
import json
import os
from pathlib import Path
import statistics
import time
import wave

import numpy as np
import sherpa_onnx


def main():
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--threads', type=int, nargs='+', default=[4, 2, 6])
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--engine', choices=['nemotron', 'parakeet', 'both'], default='both')
    parser.add_argument('--compare-padding', action='store_true', help='Compare old and reused capture padding (Nemotron only).')
    parser.add_argument('--short-only', action='store_true')
    args = parser.parse_args()
    if args.repeats < 1 or any(t < 1 for t in args.threads):
        parser.error('Thread counts and repeats must be positive.')
    app_dir = Path(os.environ.get('APPDATA', Path.home() / '.config')) / 'com.typr.app'
    clips = []
    for filename, limit in [('clip1_5s.wav', 0), ('clip2_30s.wav', 20)]:
        if args.short_only and limit: continue
        with wave.open(str(root / 'scripts/test_clips' / filename), 'rb') as wav:
            assert wav.getnchannels() == 1 and wav.getsampwidth() == 2
            rate = wav.getframerate()
            data = np.frombuffer(wav.readframes(rate * limit if limit else wav.getnframes()), dtype=np.int16).astype(np.float32) / 32768.0
            clips.append((filename, rate, data))
    baseline = {}
    for engine in ['nemotron', 'parakeet']:
        if args.engine not in ['both', engine]:
            continue
        folder = app_dir / ('nemotron-3.5-asr-streaming-0.6b-int8' if engine == 'nemotron' else 'parakeet-tdt-0.6b-v3-int8')
        for threads in args.threads:
            filenames = {name: str(folder / filename) for name, filename in [('encoder', 'encoder.int8.onnx'), ('decoder', 'decoder.int8.onnx'), ('joiner', 'joiner.int8.onnx'), ('tokens', 'tokens.txt')]}
            factory = sherpa_onnx.OnlineRecognizer if engine == 'nemotron' else sherpa_onnx.OfflineRecognizer
            recognizer = factory.from_transducer(**filenames, num_threads=threads,
                decoding_method='greedy_search', max_active_paths=8,
                model_type='' if engine == 'nemotron' else 'nemo_transducer')
            for filename, rate, data in clips:
                modes = ['extra', 'reuse'] if args.compare_padding and engine == 'nemotron' else ['reference']
                for padding_mode in modes:
                    wave_data = np.concatenate([data, np.zeros(round(rate * 0.4), dtype=np.float32)]) if padding_mode != 'reference' else data
                    padding = 20_000
                    if padding_mode == 'reuse' and rate == 16_000:
                        nonzero = np.flatnonzero(wave_data)
                        trailing = len(wave_data) - (int(nonzero[-1]) + 1 if len(nonzero) else 0)
                        padding = max(0, padding - trailing)
                    timings = []
                    for repeat in range(args.repeats + 1):
                        started = time.perf_counter()
                        stream = recognizer.create_stream()
                        if engine == 'nemotron':
                            stream.set_option('language', 'en')
                        stream.accept_waveform(rate, wave_data)
                        if engine == 'nemotron':
                            stream.accept_waveform(16000, np.zeros(padding, dtype=np.float32))
                            stream.input_finished()
                            while recognizer.is_ready(stream):
                                recognizer.decode_stream(stream)
                            transcript = recognizer.get_result(stream)
                        else:
                            recognizer.decode_stream(stream)
                            transcript = stream.result.text
                        elapsed = time.perf_counter() - started
                        key = (engine, filename)
                        baseline.setdefault(key, transcript)
                        if transcript != baseline[key]:
                            raise RuntimeError(f'Transcript changed: {engine}, {filename}, {threads} threads')
                        if repeat:
                            timings.append(elapsed)
                    print(json.dumps({'engine': engine, 'threads': threads, 'clip': filename, 'padding': padding_mode,
                        'median_seconds': round(statistics.median(timings), 4),
                        'seconds': [round(t, 4) for t in timings], 'matches_baseline': True}), flush=True)
                del stream
            del recognizer
            gc.collect()


if __name__ == '__main__':
    main()
