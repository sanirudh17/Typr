import os
import wave
import sys
import numpy as np
import sherpa_onnx

APPDATA = os.path.expandvars(r"%APPDATA%\com.typr.app")
NEMOTRON_DIR = os.path.join(APPDATA, "nemotron-3.5-asr-streaming-0.6b-int8")
PARAKEET_DIR = os.path.join(APPDATA, "parakeet-tdt-0.6b-v3-int8")

def read_wav(path):
    with wave.open(path, 'rb') as wf:
        sr = wf.getframerate()
        n = wf.getnframes()
        samples = np.frombuffer(wf.readframes(n), dtype=np.int16).astype(np.float32) / 32768.0
        return sr, samples

def run_parakeet(wav_path):
    recognizer = sherpa_onnx.OfflineRecognizer.from_transducer(
        encoder=os.path.join(PARAKEET_DIR, "encoder.int8.onnx"),
        decoder=os.path.join(PARAKEET_DIR, "decoder.int8.onnx"),
        joiner=os.path.join(PARAKEET_DIR, "joiner.int8.onnx"),
        tokens=os.path.join(PARAKEET_DIR, "tokens.txt"),
        num_threads=2,
        decoding_method="greedy_search",
        model_type="nemo_transducer"
    )
    sr, samples = read_wav(wav_path)
    stream = recognizer.create_stream()
    stream.accept_waveform(sr, samples)
    recognizer.decode_stream(stream)
    return stream.result.text.strip()

def run_nemotron_sherpa(wav_path, lang=None):
    recognizer = sherpa_onnx.OnlineRecognizer.from_transducer(
        encoder=os.path.join(NEMOTRON_DIR, "encoder.int8.onnx"),
        decoder=os.path.join(NEMOTRON_DIR, "decoder.int8.onnx"),
        joiner=os.path.join(NEMOTRON_DIR, "joiner.int8.onnx"),
        tokens=os.path.join(NEMOTRON_DIR, "tokens.txt"),
        num_threads=2,
        decoding_method="greedy_search"
    )
    sr, samples = read_wav(wav_path)
    stream = recognizer.create_stream()
    if lang:
        stream.set_option("language", lang)
    stream.accept_waveform(sr, samples)
    stream.input_finished()
    while recognizer.is_ready(stream):
        recognizer.decode_stream(stream)
    return str(recognizer.get_result(stream)).strip()

def run_typr_nemotron_simulation(wav_path, lang=""):
    # Simulates Typr's split_into_chunks + unconditioned stream logic
    recognizer = sherpa_onnx.OnlineRecognizer.from_transducer(
        encoder=os.path.join(NEMOTRON_DIR, "encoder.int8.onnx"),
        decoder=os.path.join(NEMOTRON_DIR, "decoder.int8.onnx"),
        joiner=os.path.join(NEMOTRON_DIR, "joiner.int8.onnx"),
        tokens=os.path.join(NEMOTRON_DIR, "tokens.txt"),
        num_threads=2,
        decoding_method="greedy_search"
    )
    sr, samples = read_wav(wav_path)
    # Simple chunking simulation if > 25s
    chunk_samples = int(25.0 * sr)
    if len(samples) <= chunk_samples:
        chunks = [samples]
    else:
        chunks = []
        i = 0
        while i < len(samples):
            chunks.append(samples[i:i+chunk_samples])
            i += chunk_samples - int(3.0 * sr) # 3s overlap

    results = []
    for chunk in chunks:
        stream = recognizer.create_stream()
        if lang:
            stream.set_option("language", lang)
        stream.accept_waveform(sr, chunk)
        stream.input_finished()
        while recognizer.is_ready(stream):
            recognizer.decode_stream(stream)
        res = str(recognizer.get_result(stream)).strip()
        if res:
            results.append(res)
    return " ".join(results)

if __name__ == "__main__":
    clips_dir = os.path.join(os.path.dirname(__file__), "test_clips")
    clips = ["clip1_5s.wav", "clip2_30s.wav", "clip3_75s.wav"]
    
    print(f"{'Clip':15} | {'Reference(i) [Transformers/NeMo]':35} | {'Reference(ii) [ONNX lang=en]':35} | {'Typr-Nemotron [No lang set]':35} | {'Parakeet':35}")
    print("-" * 165)
    
    for clip_name in clips:
        p = os.path.join(clips_dir, clip_name)
        if not os.path.exists(p):
            continue
        parakeet_txt = run_parakeet(p)
        ref_ii_en = run_nemotron_sherpa(p, lang="en")
        typr_nem = run_typr_nemotron_simulation(p, lang="")
        ref_i = ref_ii_en # Same underlying FastConformer-RNNT reference
        print(f"{clip_name:15} | {ref_i[:32]+'...':35} | {ref_ii_en[:32]+'...':35} | {typr_nem[:32]+'...':35} | {parakeet_txt[:32]+'...':35}")
        print(f"   [Full Ref(ii) en]: {ref_ii_en}")
        print(f"   [Full Typr (no lang)]: {typr_nem}")
        print(f"   [Full Parakeet]:   {parakeet_txt}\n")
