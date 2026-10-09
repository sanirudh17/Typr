//! Local Parakeet transcription on the CPU.
//!
//! Unlike the Whisper path this runs IN-PROCESS: no sidecar binary, no HTTP server, no CUDA.
//! That is the point of this engine — it works on machines with no NVIDIA GPU, at the cost of
//! being slower than Whisper on machines that have one.
//!
//! Two behaviours here were forced by measurement rather than chosen:
//!
//! - **The recognizer is cached.** Loading the model takes 4-6 seconds and is not disk-cache
//!   (identical on repeat runs), so building one per dictation would add that to every single
//!   transcription. It is built once and reused until the model variant changes.
//! - **Long audio is chunked.** A single decode of an 82-second recording silently dropped a
//!   whole clause while the same audio in 26-second pieces transcribed it correctly. See
//!   `audio_chunker`.

use crate::audio_chunker;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Directory name for a model variant, under the app data dir.
///
/// v3 is multilingual (25 European languages, auto-detected); v2 is English-only and faster.
/// Anything unrecognized falls back to v3 so a stale setting cannot point at a directory that
/// will never exist.
pub fn model_dir_name(variant: &str) -> &'static str {
    match variant {
        "v2" => "parakeet-tdt-0.6b-v2-int8",
        _ => "parakeet-tdt-0.6b-v3-int8",
    }
}

/// Release archive for a variant, from the sherpa-onnx model zoo.
pub fn model_download_url(variant: &str) -> String {
    format!(
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-{}.tar.bz2",
        model_dir_name(variant)
    )
}

/// The four files a sherpa-onnx offline transducer needs. Checked up front so a missing
/// download surfaces as a clear error instead of a failure deep inside the C API.
pub fn model_files_present(model_dir: &Path) -> bool {
    ["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"]
        .iter()
        .all(|f| model_dir.join(f).is_file())
}

/// The loaded model, kept alive between dictations.
///
/// Keyed by directory so switching variants rebuilds rather than silently transcribing with
/// the previous model. `sherpa_onnx::OfflineRecognizer` owns C resources and is not `Sync`,
/// so access is serialized — transcriptions are sequential anyway, one per dictation.
static RECOGNIZER: Mutex<Option<(PathBuf, sherpa_onnx::OfflineRecognizer)>> = Mutex::new(None);

fn cpu_threads(available: usize) -> usize {
    // On the six-core reference machine, six threads shortened the 20-second
    // probe by 12% with identical beam-8 output. Keep four on smaller machines.
    if available >= 12 { 6 } else { available.clamp(1, 4) }
}

fn build_recognizer(model_dir: &Path) -> Result<sherpa_onnx::OfflineRecognizer, String> {
    let threads = cpu_threads(std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4));
    build_recognizer_with_threads(model_dir, threads)
}

fn build_recognizer_with_threads(model_dir: &Path, num_threads: usize) -> Result<sherpa_onnx::OfflineRecognizer, String> {
    sherpa_onnx::OfflineRecognizer::create(&recognizer_config(model_dir, num_threads))
        .ok_or_else(|| "Failed to load the Parakeet model.".to_string())
}

fn recognizer_config(model_dir: &Path, num_threads: usize) -> sherpa_onnx::OfflineRecognizerConfig {
    let mut config = sherpa_onnx::OfflineRecognizerConfig::default();
    config.model_config.transducer.encoder =
        Some(model_dir.join("encoder.int8.onnx").to_string_lossy().into_owned());
    config.model_config.transducer.decoder =
        Some(model_dir.join("decoder.int8.onnx").to_string_lossy().into_owned());
    config.model_config.transducer.joiner =
        Some(model_dir.join("joiner.int8.onnx").to_string_lossy().into_owned());
    config.model_config.tokens =
        Some(model_dir.join("tokens.txt").to_string_lossy().into_owned());
    config.model_config.num_threads = num_threads as i32;
    config.model_config.model_type = Some("nemo_transducer".to_string());
    println!("[Typr] Parakeet CPU threads={num_threads}, decoder=greedy_search, target_rms=0.20");
    // The previous beam-8 choice improved a small keyword score but silently
    // omitted complete clauses in the retained fast/normal microphone clips.
    // Native A/B with identical audio recovered those clauses using greedy
    // decoding at 0.20 speech RMS and also shortened inference. Keep this policy
    // specific to Parakeet TDT; Whisper needs its own beam decoder.
    config.decoding_method = Some("greedy_search".to_string());
    config
}

fn prepare_decoder_audio(samples: &mut [f32], sample_rate: u32) {
    // The shared capture target is 0.08. Parakeet's int8 encoder was sensitive
    // to that level in the actual recordings: 0.20 restored the overlay and
    // final-word clauses. Limit additional amplification to 2.5x, leave silence
    // alone, and use the same soft limiter. Other engines keep their tested
    // levels; raising Nemotron's level did not consistently improve recognition.
    crate::audio::normalize_target_rms(samples, sample_rate, 0.20, 2.5);
    crate::audio::apply_tanh_limiter(samples, 0.75, 0.95);
}

fn timed_words(result: &sherpa_onnx::OfflineRecognizerResult, offset: f32) -> Option<Vec<audio_chunker::TimedWord>> {
    let timestamps = result.timestamps.as_ref()?;
    if timestamps.len() != result.tokens.len() || timestamps.iter().any(|t| !t.is_finite() || *t < 0.0)
        || timestamps.windows(2).any(|pair| pair[1] < pair[0]) { return None; }
    let mut words = Vec::new();
    let mut word = String::new();
    let mut start = 0.0;
    for (index, token) in result.tokens.iter().enumerate() {
        for ch in token.replace('\u{2581}', " ").chars() {
            if ch.is_whitespace() {
                if !word.is_empty() {
                    words.push(audio_chunker::TimedWord { text: std::mem::take(&mut word), start });
                }
            } else {
                if word.is_empty() { start = timestamps[index] + offset; }
                word.push(ch);
            }
        }
    }
    if !word.is_empty() { words.push(audio_chunker::TimedWord { text: word, start }); }
    let displayed: Vec<_> = result.text.split_whitespace().collect();
    if displayed.len() != words.len() || words.iter().zip(&displayed).any(|(word, text)| word.text != *text) { return None; }
    Some(words)
}

fn decode_chunk(recognizer: &sherpa_onnx::OfflineRecognizer, audio: &[f32],
    offset: f32, overlaps_previous: bool,
) -> Result<audio_chunker::TimedChunkText, String> {
    let stream = recognizer.create_stream();
    stream.accept_waveform(16_000, audio);
    recognizer.decode(&stream);
    let result = stream.get_result().ok_or("Parakeet returned no chunk result; refusing partial text")?;
    if result.text.trim().is_empty() && !crate::audio::is_silent_capture(audio, 16_000, 1.0) {
        return Err("Parakeet returned empty text for audible audio; refusing partial transcription".into());
    }
    let mut words = timed_words(&result, 0.0);
    let mut text = result.text.trim().to_string();
    if let Some(words) = words.as_mut() {
        text = words.iter().map(|word| word.text.as_str()).collect::<Vec<_>>().join(" ");
        for word in words { word.start += offset; }
    }
    Ok(audio_chunker::TimedChunkText { text, words, overlaps_previous })
}

/// Buffered streaming for the offline TDT export: complete, overlapped chunks
/// are decoded during capture. No encoder cache or truncated 2-second decode is
/// assumed. Each chunk uses deterministic batch processing and keeps its context.
pub struct ParakeetBufferedDecoder {
    model_dir: PathBuf,
    samples: Vec<f32>,
    planning_samples: Vec<f32>,
    planning_filter: crate::audio::StreamingPreprocessor,
    planner: audio_chunker::ChunkPlanner,
    parts: Vec<audio_chunker::TimedChunkText>,
    completed_chunks: usize,
    finished: bool,
}

impl ParakeetBufferedDecoder {
    pub fn new(model_dir: &Path) -> Result<Self, String> {
        prewarm(model_dir)?;
        Ok(Self { model_dir: model_dir.to_path_buf(), samples: Vec::new(), planning_samples: Vec::new(),
            planning_filter: crate::audio::StreamingPreprocessor::new(16_000, 0.0),
            planner: audio_chunker::ChunkPlanner::new(16_000), parts: Vec::new(),
            completed_chunks: 0, finished: false })
    }

    /// Accept continuously resampled mono 16 kHz raw samples. Finishing drains
    /// the last incomplete frame and decodes the remaining chunk exactly once.
    pub fn push(&mut self, samples: &[f32], finished: bool) -> Result<(), String> {
        if self.finished { return Err("Parakeet input was already finished".into()); }
        self.samples.extend_from_slice(samples);
        let mut planning = samples.to_vec();
        self.planning_filter.highpass_only(&mut planning);
        self.planning_samples.extend(planning);
        for (range, overlaps) in self.planner.advance(&self.planning_samples, finished) {
            let offset = range.start as f32 / 16_000.0;
            let mut audio = self.samples[range].to_vec();
            crate::audio::process_speech_audio_chain(&mut audio, 16_000, crate::audio::FIXED_INPUT_GAIN_DB);
            // The recognizer accepts float PCM directly. Retain the capture's
            // precision instead of quantizing an intermediate review WAV.
            audio.resize(audio.len() + 6_400, 0.0);
            prepare_decoder_audio(&mut audio, 16_000);
            let mut guard = RECOGNIZER.lock().map_err(|_| "Parakeet model lock poisoned; restart Typr.")?;
            if !matches!(&*guard, Some((dir, _)) if dir == &self.model_dir) {
                *guard = Some((self.model_dir.clone(), build_recognizer(&self.model_dir)?));
            }
            let recognizer = &guard.as_ref().expect("just built").1;
            self.parts.push(decode_chunk(recognizer, &audio, offset, overlaps)?);
            if !finished { self.completed_chunks += 1; }
        }
        self.finished = finished;
        Ok(())
    }

    pub fn text(&self) -> String { audio_chunker::merge_timed_chunks(&self.parts) }
    pub fn completed_chunks(&self) -> usize { self.completed_chunks }
}

pub struct ParakeetBufferedResult {
    pub text: String,
    pub completed_chunks: usize,
    pub tail_ms: u128,
}

pub struct ParakeetLiveSession {
    pub model_dir: PathBuf,
    finish_signal: std::sync::Arc<std::sync::atomic::AtomicBool>,
    cancel_signal: std::sync::Arc<std::sync::atomic::AtomicBool>,
    completed_chunks: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    worker: Option<std::thread::JoinHandle<Result<ParakeetBufferedResult, String>>>,
}

impl ParakeetLiveSession {
    pub fn completed_chunks(&self) -> usize {
        self.completed_chunks.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn finish(mut self) -> Result<ParakeetBufferedResult, String> {
        self.finish_signal.store(true, std::sync::atomic::Ordering::Release);
        let worker = self.worker.take().ok_or("Parakeet worker was already joined")?;
        worker.thread().unpark();
        worker.join().map_err(|_| "Parakeet streaming worker panicked".to_string())?
    }
}

impl Drop for ParakeetLiveSession {
    fn drop(&mut self) {
        self.cancel_signal.store(true, std::sync::atomic::Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

pub fn start_live_session(model_dir: &Path,
    recorder: std::sync::Arc<Mutex<crate::audio::AudioRecorder>>,
) -> Result<ParakeetLiveSession, String> {
    let mut decoder = ParakeetBufferedDecoder::new(model_dir)?;
    let source_rate = recorder.lock().map_err(|_| "Audio recorder lock poisoned")?.source_sample_rate();
    let finish_signal = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_signal = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let completed_chunks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (finish, cancel, completed) = (finish_signal.clone(), cancel_signal.clone(), completed_chunks.clone());
    let worker = std::thread::Builder::new().name("parakeet-buffered-stream".into()).spawn(move || {
        let mut last_read = 0;
        let mut resampler = crate::audio::StreamingResampler::new(source_rate, 16_000);
        loop {
            if cancel.load(std::sync::atomic::Ordering::Acquire) { return Err("Parakeet capture cancelled".into()); }
            // The recorder has stopped before finish is signalled. Read again
            // after seeing that signal so no callback tail can be overlooked.
            let finishing = finish.load(std::sync::atomic::Ordering::Acquire);
            let tail_started = std::time::Instant::now();
            let (raw, end, rate, channels) = recorder.lock().map_err(|_| "Audio recorder lock poisoned")?
                .get_raw_samples_from(last_read);
            if rate != source_rate || channels == 0 { return Err("Audio format changed during dictation".into()); }
            last_read = end;
            let mono: Vec<f32> = raw.chunks_exact(channels as usize)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32).collect();
            let mut samples = resampler.process(&mono);
            if finishing { samples.extend(resampler.finish()); }
            decoder.push(&samples, finishing)?;
            completed.store(decoder.completed_chunks(), std::sync::atomic::Ordering::Relaxed);
            if finishing {
                return Ok(ParakeetBufferedResult { text: decoder.text(),
                    completed_chunks: decoder.completed_chunks(), tail_ms: tail_started.elapsed().as_millis() });
            }
            std::thread::park_timeout(std::time::Duration::from_millis(100));
        }
    }).map_err(|e| format!("Could not start Parakeet stream: {e}"))?;
    Ok(ParakeetLiveSession { model_dir: model_dir.to_path_buf(), finish_signal, cancel_signal, completed_chunks,
        worker: Some(worker) })
}

/// Drop the cached model, freeing ~640 MB. NOT currently wired up: the model is deliberately
/// kept loaded across engine switches so returning to Parakeet stays fast (rebuilding costs
/// ~4-6s). Retained for a future memory-pressure path that would trade that latency back.
pub fn release_model() {
    if let Ok(mut guard) = RECOGNIZER.lock() {
        *guard = None;
    }
}

/// Build and cache the recognizer WITHOUT transcribing, so the first dictation after Parakeet
/// is selected doesn't pay the ~4-6s model load. That load otherwise happens lazily inside the
/// first `transcribe_parakeet` and is invisible in the "completed in" log (whose timer starts
/// after the build) — it was the "first dictation 8s, second 1.5s" bug. Idempotent: a no-op
/// when the recognizer is already built for `model_dir`. Blocking and CPU-bound — call it from a
/// blocking task, never on an async worker.
pub fn prewarm(model_dir: &Path) -> Result<(), String> {
    if !model_files_present(model_dir) {
        return Err(format!(
            "Parakeet model not found in {}. Download it from the Engine tab.",
            model_dir.display()
        ));
    }
    let mut guard = RECOGNIZER
        .lock()
        .map_err(|_| "Parakeet model lock poisoned; restart Typr.".to_string())?;
    let needs_build = !matches!(&*guard, Some((dir, _)) if dir == model_dir);
    if needs_build {
        let started = std::time::Instant::now();
        println!(
            "[Typr] Prewarming Parakeet model {}...",
            model_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        );
        *guard = Some((model_dir.to_path_buf(), build_recognizer(model_dir)?));
        println!("[Typr] Parakeet model ready in {:?}", started.elapsed());
    }
    Ok(())
}

/// Transcribe a 16 kHz mono WAV. `audio.rs` already produces exactly that format.
pub async fn transcribe_parakeet(
    model_dir: &PathBuf,
    audio_path: &PathBuf,
) -> Result<String, String> {
    if !model_files_present(model_dir) {
        return Err(format!(
            "Parakeet model not found in {}. Download it from the Engine tab.",
            model_dir.display()
        ));
    }

    let model_dir = model_dir.clone();
    let audio_path = audio_path.clone();

    // sherpa-onnx is synchronous and CPU-bound; keeping it off the async workers stops a long
    // dictation from stalling the runtime.
    tokio::task::spawn_blocking(move || {
        let mut reader = hound::WavReader::open(&audio_path)
            .map_err(|e| format!("Failed to read audio file: {}", e))?;
        if reader.spec().channels != 1 || reader.spec().sample_rate != 16_000
            || reader.spec().bits_per_sample != 16 || reader.spec().sample_format != hound::SampleFormat::Int {
            return Err("Parakeet requires a mono 16 kHz PCM16 prepared WAV.".into());
        }
        let sample_rate = reader.spec().sample_rate;
        let num_samples = reader.len() as usize;
        let mut samples = Vec::with_capacity(num_samples);
        for s in reader.samples::<i16>() {
            let v = s.map_err(|e| format!("Failed to decode audio sample: {}", e))?;
            samples.push(v as f32 / 32768.0);
        }
        prepare_decoder_audio(&mut samples, sample_rate);

        let mut guard = RECOGNIZER
            .lock()
            .map_err(|_| "Parakeet model lock poisoned; restart Typr.".to_string())?;
        let needs_build = !matches!(&*guard, Some((dir, _)) if dir == &model_dir);
        if needs_build {
            *guard = Some((model_dir.clone(), build_recognizer(&model_dir)?));
        }
        let recognizer = &guard.as_ref().expect("just built").1;

        let chunks = audio_chunker::split_into_chunks(&samples, sample_rate);
        // Logged because the Whisper path announces itself loudly and this one did not, which
        // made a run on the wrong engine impossible to spot in the console.
        println!(
            "[Typr] Parakeet transcribing {:.1}s in {} chunk(s), model {}",
            samples.len() as f32 / sample_rate as f32,
            chunks.len(),
            model_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        );
        let started = std::time::Instant::now();
        let mut parts = Vec::new();
        for chunk in &chunks {
            let start = (chunk.samples.as_ptr() as usize - samples.as_ptr() as usize) / std::mem::size_of::<f32>();
            parts.push(decode_chunk(recognizer, chunk.samples, start as f32 / sample_rate as f32, chunk.overlaps_previous)?);
        }
        let merged = audio_chunker::merge_timed_chunks(&parts);
        println!("[Typr] Parakeet completed in {:?}", started.elapsed());
        Ok(merged)
    })
    .await
    .map_err(|e| format!("Parakeet task panicked: {}", e))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(text: &str, tokens: &[&str], timestamps: &[f32]) -> sherpa_onnx::OfflineRecognizerResult {
        sherpa_onnx::OfflineRecognizerResult { text: text.into(), tokens: tokens.iter().map(|s| (*s).into()).collect(),
            timestamps: Some(timestamps.to_vec()), durations: None }
    }

    #[test]
    fn token_alignment_keeps_words_punctuation_and_absolute_times() {
        let words = timed_words(&result("Keep all final words.", &[" Keep", " all", " fin", "al", " words", "."],
            &[0., 0.2, 0.4, 0.48, 0.7, 0.8]), 22.).unwrap();
        assert_eq!(words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["Keep", "all", "final", "words."]);
        assert_eq!(words[2].start, 22.4);
        let repeated = timed_words(&result("very very", &[" very", " very"], &[0., 0.2]), 0.).unwrap();
        assert_eq!(repeated.len(), 2);
    }

    #[test]
    fn unreliable_token_alignment_uses_full_text() {
        for candidate in [result("keep words", &[" keep", " words"], &[0.]),
            result("keep words", &[" keep", " words"], &[0., f32::NAN]),
            result("keep words", &[" keep", " words"], &[0.2, 0.1]),
            result("keep ALL words", &[" keep", " words"], &[0., 0.2])] {
            assert!(timed_words(&candidate, 0.).is_none());
        }
    }

    #[test]
    fn buffered_worker_drains_unread_stereo_tail() {
        let model = dirs::config_dir().unwrap().join("com.typr.app").join(model_dir_name("v3"));
        let path = PathBuf::from("../scripts/test_clips/clip1_5s.wav");
        if !model_files_present(&model) || !path.exists() { return; }
        let (samples, rate) = audio_chunker::read_wav_samples(&path).unwrap();
        let mono = crate::audio::resample(&samples, rate, 48_000);
        let stereo: Vec<_> = mono.iter().flat_map(|&s| [s, s]).collect();
        let mut recorder = crate::audio::AudioRecorder::new();
        recorder.set_source_format_for_test(48_000, 2);
        let recorder = std::sync::Arc::new(Mutex::new(recorder));
        let session = start_live_session(&model, recorder.clone()).unwrap();
        // The last second can arrive after the worker's last read. finish must
        // consume the entire buffer and the resampler's fractional/filter tail.
        let split = stereo.len().saturating_sub(96_000);
        recorder.lock().unwrap().push_raw_samples_for_test(&stereo[..split]);
        recorder.lock().unwrap().push_raw_samples_for_test(&stereo[split..]);
        let text = session.finish().unwrap().text;
        assert!(text.trim_end_matches(['.', ' ']).to_lowercase().ends_with("would like to make"), "Lost tail: {text}");
        assert_eq!(recorder.lock().unwrap().get_raw_samples_from(0).0.len(), stereo.len());
    }

    #[test]
    fn buffered_worker_decodes_before_stop_and_keeps_long_final_sentence() {
        let model = dirs::config_dir().unwrap().join("com.typr.app").join(model_dir_name("v3"));
        let path = PathBuf::from("../scripts/test_clips/clip3_75s.wav");
        if !model_files_present(&model) || !path.exists() { return; }
        let (samples, rate) = audio_chunker::read_wav_samples(&path).unwrap();
        let mut recorder = crate::audio::AudioRecorder::new();
        recorder.set_source_format_for_test(rate, 1);
        let recorder = std::sync::Arc::new(Mutex::new(recorder));
        let session = start_live_session(&model, recorder.clone()).unwrap();
        let split = 30 * rate as usize;
        recorder.lock().unwrap().push_raw_samples_for_test(&samples[..split]);
        let started = std::time::Instant::now();
        while session.completed_chunks() == 0 && started.elapsed().as_secs() < 45 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(session.completed_chunks() > 0, "No chunk decoded during capture");
        recorder.lock().unwrap().push_raw_samples_for_test(&samples[split..]);
        let result = session.finish().unwrap();
        let lower = result.text.to_lowercase();
        assert!(lower.ends_with("some changes that you would like to make."), "Lost final words: {lower}");
        assert!(lower.contains("speech engines before deployment"), "Lost seam words: {lower}");
        assert!(!lower.contains("beach engines"), "Replayed seam was duplicated: {lower}");
        assert_eq!(recorder.lock().unwrap().get_raw_samples_from(0).0.len(), samples.len());
    }

    #[tokio::test]
    async fn production_batch_keeps_long_dictation_items_seams_and_ending() {
        let model = dirs::config_dir().unwrap().join("com.typr.app").join(model_dir_name("v3"));
        let source = PathBuf::from("../scripts/test_clips/clip3_75s.wav");
        if !model_files_present(&model) || !source.exists() { return; }
        let (raw, rate) = audio_chunker::read_wav_samples(&source).unwrap();
        let mut samples = crate::audio::resample(&raw, rate, 16_000);
        crate::audio::process_speech_audio_chain(&mut samples, 16_000, crate::audio::FIXED_INPUT_GAIN_DB);
        samples.resize(samples.len() + 6_400, 0.0);
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("typr-parakeet-tail-{}-{nonce}.wav", std::process::id()));
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
        std::io::Write::write_all(&mut file, &audio_chunker::samples_to_wav_bytes(&samples, 16_000).unwrap()).unwrap();
        drop(file);
        let result = transcribe_parakeet(&model, &path).await;
        let _ = std::fs::remove_file(&path);
        let lower = result.unwrap().to_lowercase();
        assert_eq!(lower.matches(" item").count(), 10, "Lost or repeated list item: {lower}");
        assert_eq!(lower.matches("special language tags").count(), 1, "Repeated seam: {lower}");
        assert_eq!(lower.matches("speech engines before deployment").count(), 1, "Lost or repeated seam: {lower}");
        assert!(!lower.contains("beach engines"), "Duplicated alternate seam: {lower}");
        assert!(lower.ends_with("some changes that you would like to make."), "Lost final words: {lower}");
    }

    #[test]
    fn decoder_policy_uses_greedy_for_tdt() {
        let config = recognizer_config(Path::new("fixture"), 4);
        assert_eq!(config.decoding_method.as_deref(), Some("greedy_search"));
        assert_eq!(config.model_config.model_type.as_deref(), Some("nemo_transducer"));
    }

    #[test]
    fn decoder_audio_boost_is_bounded_and_preserves_silence() {
        let mut normal = vec![0.08; 1600];
        prepare_decoder_audio(&mut normal, 16000);
        assert!((crate::audio::measure_p95_frame_rms(&normal, 16000) - 0.20).abs() < 1e-5);
        let mut quiet = vec![0.001; 1600];
        prepare_decoder_audio(&mut quiet, 16000);
        assert!((quiet[0] - 0.0025).abs() < 1e-6);
        let mut silent = vec![0.0; 1600];
        prepare_decoder_audio(&mut silent, 16000);
        assert!(silent.iter().all(|&s| s == 0.0));
        let mut peaks = vec![0.08; 1600];
        peaks[0] = 1.0;
        prepare_decoder_audio(&mut peaks, 16000);
        assert!(peaks.iter().all(|s| s.abs() <= 0.95));
    }

    #[test]
    fn thread_policy_scales_without_oversubscribing_small_machines() {
        assert_eq!(cpu_threads(1), 1);
        assert_eq!(cpu_threads(2), 2);
        assert_eq!(cpu_threads(8), 4);
        assert_eq!(cpu_threads(12), 6);
        assert_eq!(cpu_threads(32), 6);
    }

    #[test]
    fn reference_transcripts_match_at_four_and_six_threads() {
        let model_dir = dirs::config_dir().unwrap().join("com.typr.app").join(model_dir_name("v3"));
        let clips: Vec<_> = ["clip1_5s.wav", "clip2_30s.wav", "clip3_75s.wav"]
            .iter().map(|name| PathBuf::from("../scripts/test_clips").join(name)).collect();
        if !model_files_present(&model_dir) || !clips.iter().all(|p| p.exists()) { return; }
        let mut baseline = Vec::new();
        for threads in [4, 6] {
            let recognizer = build_recognizer_with_threads(&model_dir, threads).unwrap();
            for (index, path) in clips.iter().enumerate() {
                let (samples, rate) = audio_chunker::read_wav_samples(path).unwrap();
                let mut parts = Vec::new();
                for chunk in audio_chunker::split_into_chunks(&samples, rate) {
                    let stream = recognizer.create_stream();
                    stream.accept_waveform(rate as i32, chunk.samples);
                    recognizer.decode(&stream);
                    let text = stream.get_result().unwrap().text.trim().to_string();
                    if !text.is_empty() { parts.push((text, chunk.overlaps_previous)); }
                }
                let text = audio_chunker::merge_chunk_texts(&parts);
                assert!(!text.is_empty());
                if threads == 4 { baseline.push(text); }
                else { assert_eq!(text, baseline[index], "Thread tuning changed {}", path.display()); }
            }
        }
    }

    #[test]
    fn test_model_dir_name_maps_variants() {
        assert_eq!(model_dir_name("v3"), "parakeet-tdt-0.6b-v3-int8");
        assert_eq!(model_dir_name("v2"), "parakeet-tdt-0.6b-v2-int8");
        // Unknown or empty falls back to the default variant, so a stale setting can never
        // point at a directory that will never exist.
        assert_eq!(model_dir_name(""), "parakeet-tdt-0.6b-v3-int8");
        assert_eq!(model_dir_name("bogus"), "parakeet-tdt-0.6b-v3-int8");
    }

    #[test]
    fn test_model_download_url_per_variant() {
        assert!(model_download_url("v3").ends_with("parakeet-tdt-0.6b-v3-int8.tar.bz2"));
        assert!(model_download_url("v2").ends_with("parakeet-tdt-0.6b-v2-int8.tar.bz2"));
        assert!(model_download_url("v3").starts_with("https://"));
        assert!(model_download_url("bogus").ends_with("parakeet-tdt-0.6b-v3-int8.tar.bz2"));
    }

    #[test]
    fn test_model_files_present_false_for_missing_dir() {
        assert!(!model_files_present(Path::new("does-not-exist-anywhere")));
    }

    #[tokio::test]
    async fn test_missing_model_errors_clearly() {
        let r = transcribe_parakeet(
            &PathBuf::from("does-not-exist-anywhere"),
            &PathBuf::from("nope.wav"),
        )
        .await;
        assert!(r.is_err());
        let e = r.unwrap_err();
        assert!(e.contains("model"), "error should name the model problem, got: {}", e);
    }

    #[test]
    fn test_parakeet_seam_recovery_retains_all_words() {
        // Simulating the Parakeet seam:
        // Chunk 0 ends at forced cut: "First section with item 1 and item 2 and item 3."
        // Chunk 1 begins within 3s overlap: "item 3. Item 4 and item 5 and final changes."
        // Under the old trim_overlap_tokens, blind 3s trimming dropped words unconditionally.
        // Under merge_chunk_texts, the seam reconciles "item 3" duplicate and preserves every word.
        let parts = vec![
            ("First section with item 1 and item 2 and item 3.".to_string(), false),
            ("item 3. Item 4 and item 5 and final changes.".to_string(), true),
        ];
        let merged = audio_chunker::merge_chunk_texts(&parts);
        assert_eq!(
            merged,
            "First section with item 1 and item 2 and item 3. Item 4 and item 5 and final changes."
        );
    }
}
