"""Compare Nemotron finalization padding on Typr's local clips (no AI or network).

Requires sherpa-onnx and numpy in the Python environment. References are read
from the existing golden test; this script does not synthesize or alter audio.
"""
import argparse
import json
import os
from pathlib import Path
import re
import time
import wave

import numpy as np
import sherpa_onnx


def word_errors(reference, hypothesis):
    words = lambda text: re.findall(r"\w+", text.lower())
    reference, hypothesis = words(reference), words(hypothesis)
    row = list(range(len(hypothesis) + 1))
    for i, word in enumerate(reference, 1):
        next_row = [i]
        for j, heard in enumerate(hypothesis, 1):
            next_row.append(min(next_row[-1] + 1, row[j] + 1, row[j - 1] + (word != heard)))
        row = next_row
    return row[-1], len(reference)


def main():
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--model-dir', type=Path, default=Path(os.environ.get('APPDATA', Path.home() / '.config')) / 'com.typr.app' / 'nemotron-3.5-asr-streaming-0.6b-int8')
    parser.add_argument('--clips', type=Path, default=root / 'scripts/test_clips')
    parser.add_argument('--padding', type=float, nargs='+', default=[1.0, 1.25])
    args = parser.parse_args()
    source = (root / 'src-tauri/src/transcribe_nemotron.rs').read_text(encoding='utf-8')
    references = re.findall(r'"\.\./scripts/test_clips/(clip\d_\d+s\.wav)",\s*"([^"]+)"', source)
    if len(references) != 3:
        raise RuntimeError('Expected all three existing golden references.')
    modes = args.padding
    for padding in modes:
        mode = f'greedy-padding-{padding:g}s'
        load_at = time.perf_counter()
        recognizer = sherpa_onnx.OnlineRecognizer.from_transducer(
            **{name: str(args.model_dir / filename) for name, filename in [
                ('encoder', 'encoder.int8.onnx'), ('decoder', 'decoder.int8.onnx'),
                ('joiner', 'joiner.int8.onnx'), ('tokens', 'tokens.txt')]},
            num_threads=4, decoding_method='greedy_search',
        )
        print(json.dumps({'decoder': mode, 'load_seconds': round(time.perf_counter() - load_at, 3)}), flush=True)
        total_errors = total_words = 0
        for filename, reference in references:
            with wave.open(str(args.clips / filename), 'rb') as wav:
                assert wav.getnchannels() == 1 and wav.getsampwidth() == 2
                rate = wav.getframerate()
                samples = np.frombuffer(wav.readframes(wav.getnframes()), dtype=np.int16).astype(np.float32) / 32768.0
            started = time.perf_counter()
            stream = recognizer.create_stream()
            if '3.5' in str(args.model_dir):
                stream.set_option('language', 'en')
            stream.accept_waveform(rate, samples)
            while recognizer.is_ready(stream):
                recognizer.decode_stream(stream)
            stream.accept_waveform(16000, np.zeros(round(16000 * padding), dtype=np.float32))
            stream.input_finished()
            while recognizer.is_ready(stream):
                recognizer.decode_stream(stream)
            hypothesis = recognizer.get_result(stream)
            errors, count = word_errors(reference, hypothesis)
            total_errors += errors
            total_words += count
            print(json.dumps({'decoder': mode, 'clip': filename, 'errors': errors, 'words': count,
                'wer': round(errors / count, 4), 'seconds': round(time.perf_counter() - started, 3),
                'transcript': hypothesis}), flush=True)
        print(json.dumps({'decoder': mode, 'total_errors': total_errors, 'total_words': total_words,
            'wer': round(total_errors / total_words, 4)}), flush=True)


if __name__ == '__main__':
    main()
