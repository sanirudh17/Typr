//! Compare complete and incremental buffered TDT decoding on saved audio.
//! Local only; never records, uploads, changes settings, or writes app data.
use typr_lib::{audio, transcribe_parakeet::ParakeetBufferedDecoder};
fn main() -> Result<(), String> {
    let model = dirs::config_dir().ok_or("No config directory")?.join("com.typr.app")
        .join(typr_lib::transcribe_parakeet::model_dir_name("v3"));
    for path in std::env::args_os().skip(1).map(std::path::PathBuf::from) {
        let mut wav = hound::WavReader::open(&path).map_err(|e| e.to_string())?;
        let spec = wav.spec();
        if spec.channels == 0 || spec.sample_rate == 0 { return Err("Invalid audio format".into()); }
        let raw: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => wav.samples::<f32>().collect::<Result<_, _>>().map_err(|e| e.to_string())?,
            hound::SampleFormat::Int => {
                let scale = 2f32.powi(spec.bits_per_sample as i32 - 1);
                wav.samples::<i32>().map(|s| s.map(|n| n as f32 / scale))
                    .collect::<Result<_, _>>().map_err(|e| e.to_string())?
            }
        };
        if raw.is_empty() || raw.len() % spec.channels as usize != 0 || raw.iter().any(|s| !s.is_finite()) {
            return Err("Invalid audio samples".into());
        }
        let mono: Vec<f32> = raw.chunks_exact(spec.channels as usize)
            .map(|frame| frame.iter().sum::<f32>() / spec.channels as f32).collect();
        let mut complete = ParakeetBufferedDecoder::new(&model)?;
        let started = std::time::Instant::now();
        complete.push(&audio::resample(&mono, spec.sample_rate, 16_000), true)?;
        let complete_ms = started.elapsed().as_millis();
        let mut incremental = ParakeetBufferedDecoder::new(&model)?;
        let mut resampler = audio::StreamingResampler::new(spec.sample_rate, 16_000);
        for part in mono.chunks((spec.sample_rate / 10).max(1) as usize) {
            incremental.push(&resampler.process(part), false)?;
        }
        let tail_started = std::time::Instant::now();
        incremental.push(&resampler.finish(), true)?;
        let text = incremental.text();
        if text != complete.text() { return Err(format!("Incremental parity failed for {}", path.display())); }
        println!("BUFFERED REPLAY {}", serde_json::json!({
            "file":path.file_name().unwrap_or_default().to_string_lossy(),
            "seconds": mono.len() as f32 / spec.sample_rate as f32,
            "complete_decode_ms":complete_ms, "tail_decode_ms":tail_started.elapsed().as_millis(),
            "completed_chunks":incremental.completed_chunks(),"identical":true,"text":text}));
    }
    Ok(())
}
