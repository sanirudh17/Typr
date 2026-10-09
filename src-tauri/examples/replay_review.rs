//! Replay one retained microphone WAV through the installed local engines.
//! Uses production preprocessing and decoders; never records, pastes, or uploads.
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use typr_lib::{audio, cleanup, dictionary::Dictionary, transcribe_nemotron, transcribe_parakeet, vocab_correct};

struct TemporaryWav(PathBuf);

impl Drop for TemporaryWav {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

fn prepare_wav(input: &Path) -> Result<TemporaryWav, String> {
    let mut reader = hound::WavReader::open(input).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.sample_rate == 0 { return Err("Invalid WAV format".into()); }
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>()
            .collect::<Result<_, _>>().map_err(|e| e.to_string())?,
        hound::SampleFormat::Int => {
            let scale = 2f32.powi(spec.bits_per_sample as i32 - 1);
            reader.samples::<i32>().map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>().map_err(|e| e.to_string())?
        }
    };
    if raw.is_empty() || raw.len() % spec.channels as usize != 0
        || raw.iter().any(|s| !s.is_finite()) {
        return Err("WAV contains empty, incomplete, or non-finite samples".into());
    }
    // Same sequence as AudioRecorder::save_capture, including its i16 quantization.
    let mono: Vec<f32> = raw.chunks_exact(spec.channels as usize)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32).collect();
    let mut samples = audio::resample(&mono, spec.sample_rate, 16_000);
    let diagnostics = audio::process_speech_audio_chain(&mut samples, 16_000, audio::FIXED_INPUT_GAIN_DB);
    println!("REPLAY AUDIO {}", serde_json::json!({
        "source_rate": spec.sample_rate, "channels": spec.channels,
        "seconds": mono.len() as f64 / spec.sample_rate as f64,
        "post_p95_rms": diagnostics.post_rms, "clipped_samples": diagnostics.clip_count,
    }));
    samples.resize(samples.len() + 6_400, 0.0);
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let path = std::env::temp_dir().join(format!("typr-review-replay-{}-{nonce}.wav", std::process::id()));
    let file = std::fs::OpenOptions::new().write(true).create_new(true)
        .open(&path).map_err(|e| e.to_string())?;
    let temp = TemporaryWav(path);
    let mut writer = hound::WavWriter::new(std::io::BufWriter::new(file), hound::WavSpec {
        channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int,
    }).map_err(|e| e.to_string())?;
    for sample in samples {
        writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())?;
    Ok(temp)
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let input = PathBuf::from(args.next().ok_or("Usage: cargo run --example replay_review -- <raw-review.wav>")?);
    let export = match args.next() {
        Some(flag) if flag == "--prepare-only" => Some(PathBuf::from(args.next().ok_or("Missing export path")?)),
        Some(_) => return Err("Expected a raw WAV path and optional --prepare-only <new.wav>".into()),
        None => None,
    };
    if args.next().is_some() { return Err("Unexpected argument".into()); }
    let prepared = prepare_wav(&input)?;
    if let Some(export) = export {
        // Diagnostic export only: create a new file, never overwrite a recording.
        let mut output = std::fs::OpenOptions::new().write(true).create_new(true)
            .open(&export).map_err(|e| e.to_string())?;
        let mut source = std::fs::File::open(&prepared.0).map_err(|e| e.to_string())?;
        std::io::copy(&mut source, &mut output).map_err(|e| e.to_string())?;
        println!("REPLAY PREPARED {}", export.display());
        return Ok(());
    }
    let app_dir = dirs::config_dir().ok_or("No app configuration directory")?.join("com.typr.app");
    // Read only. Dictionary::load can migrate and save settings, so don't call it here.
    let dict = match std::fs::read_to_string(app_dir.join("dictionary.json")) {
        Ok(json) => serde_json::from_str::<Dictionary>(&json).map_err(|e| e.to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Dictionary::default(),
        Err(e) => return Err(e.to_string()),
    };
    let mut decoded = 0;
    for engine in ["nemotron", "parakeet"] {
        let model = app_dir.join(if engine == "nemotron" {
            transcribe_nemotron::model_dir_name("v3.5")
        } else { transcribe_parakeet::model_dir_name("v3") });
        let present = if engine == "nemotron" { transcribe_nemotron::model_files_present(&model) }
            else { transcribe_parakeet::model_files_present(&model) };
        if !present {
            println!("REPLAY SKIPPED {engine}: model is not installed");
            continue;
        }
        if engine == "nemotron" { transcribe_nemotron::prewarm(&model)?; }
        else { transcribe_parakeet::prewarm(&model)?; }
        let started = Instant::now();
        let raw = if engine == "nemotron" {
            transcribe_nemotron::transcribe_nemotron(&model, &prepared.0).await?
        } else { transcribe_parakeet::transcribe_parakeet(&model, &prepared.0).await? };
        let decode_ms = started.elapsed().as_millis();
        let corrected = vocab_correct::correct_vocabulary(&raw, &dict.vocabulary_hints);
        let corrected = dict.apply_replacements(&corrected);
        let corrected = typr_lib::email_assemble::assemble_emails(&corrected);
        println!("REPLAY RESULT {}", serde_json::json!({
            "engine": engine, "path": "batch", "decode_ms": decode_ms,
            "raw": raw, "ai_off": cleanup::cleanup_text(&corrected),
        }));
        if engine == "nemotron" { transcribe_nemotron::release_model(); }
        else { transcribe_parakeet::release_model(); }
        decoded += 1;
    }
    if decoded == 0 { return Err("Neither supported local model is installed".into()); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_stereo_review_wav_uses_exact_batch_processing() {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("typr-review-input-{}-{nonce}.wav", std::process::id()));
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
        let input = TemporaryWav(path);
        let mut writer = hound::WavWriter::new(file, hound::WavSpec {
            channels: 2, sample_rate: 44_100, bits_per_sample: 32, sample_format: hound::SampleFormat::Float,
        }).unwrap();
        let mut mono = Vec::new();
        for i in 0..44_100 {
            let left = 0.002 * (i as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin();
            let right = left * 0.5;
            writer.write_sample(left).unwrap();
            writer.write_sample(right).unwrap();
            mono.push((left + right) / 2.0);
        }
        writer.finalize().unwrap();
        let prepared = prepare_wav(&input.0).unwrap();
        let mut expected = audio::resample(&mono, 44_100, 16_000);
        audio::process_speech_audio_chain(&mut expected, 16_000, audio::FIXED_INPUT_GAIN_DB);
        expected.resize(expected.len() + 6_400, 0.0);
        let expected: Vec<i16> = expected.iter()
            .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).collect();
        let mut reader = hound::WavReader::open(&prepared.0).unwrap();
        assert_eq!((reader.spec().sample_rate, reader.spec().channels, reader.spec().bits_per_sample), (16_000, 1, 16));
        let actual = reader.samples::<i16>().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(actual, expected);
    }
}
