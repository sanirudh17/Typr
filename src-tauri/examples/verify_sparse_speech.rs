//! Synthetic word fixtures only. No microphone capture or personal documents.
//! Optional Groq requests require TYPR_LIVE_ASR_TEST=1.
use typr_lib::{audio, audio_chunker, settings::Settings, speech_detection};

#[tokio::main]
async fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 { return Err("Provide a synthetic WAV path and its expected word(s).".into()); }
    let (source, rate) = audio_chunker::read_wav_samples(&std::path::PathBuf::from(&args[0]))?;
    let source = audio::resample(&source, rate, 16000);
    let peak = source.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    let first = source.iter().position(|s| s.abs() > peak * 0.02).ok_or("Empty fixture")?.saturating_sub(1280);
    let last = (source.iter().rposition(|s| s.abs() > peak * 0.02).unwrap() + 1281).min(source.len());
    let mut seed = 99u32;
    let mut raw: Vec<f32> = (0..16000 * 24).map(|_| {
        seed=seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed as f64/u32::MAX as f64*2.0-1.0) as f32*0.00005
    }).collect();
    for (i, &s) in source[first..last].iter().enumerate() { raw[16000 * 12 + i] += s * 0.02; }
    assert!(speech_detection::contains_speech(&raw,16000)?);
    let mut legacy = raw.clone();
    audio::apply_gain_db(&mut legacy,audio::FIXED_INPUT_GAIN_DB);
    audio::remove_dc_offset(&mut legacy); audio::highpass_50hz(&mut legacy,16000);
    let p95 = audio::measure_p95_frame_rms(&legacy,16000);
    let gain = if p95 >= 1e-5 {(0.08/p95).min(40.0)} else {1.0};
    for s in &mut legacy {*s *= gain;}
    let old_overload = legacy.iter().filter(|s| s.abs() > 0.95).count();
    audio::apply_tanh_limiter(&mut legacy,0.75,0.95);
    let mut prepared = raw.clone();
    audio::process_speech_audio_chain(&mut prepared,16000,audio::FIXED_INPUT_GAIN_DB);
    let new_overload = prepared.iter().filter(|s| s.abs() > 0.95).count();
    println!("Sparse synthetic speech: legacy_overload={old_overload} new_overload={new_overload} samples={}",prepared.len());
    assert_eq!(prepared.len(),raw.len()); assert_eq!(new_overload,0);
    let app_dir = dirs::config_dir().ok_or("No config directory")?.join("com.typr.app");
    let settings = Settings::load(&app_dir);
    let temp = std::env::temp_dir().join(format!("typr-sparse-{}.wav",std::process::id()));
    let normalize = |s: &str| s.chars().filter(|c|c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let result: Result<(),String> = async {
        let mut failures=Vec::new();
        for engine in ["nemotron","parakeet","cloud"] {
            if engine=="cloud" && std::env::var("TYPR_LIVE_ASR_TEST").as_deref()!=Ok("1") {continue;}
            let model=app_dir.join(if engine=="nemotron" {"nemotron-3.5-asr-streaming-0.6b-int8"} else {"parakeet-tdt-0.6b-v3-int8"});
            if engine!="cloud" && !model.is_dir() { println!("SKIP {engine}: model unavailable");continue; }
          for (variant,audio) in [("legacy",&legacy),("candidate",&prepared)] {
            let mut padded=audio.clone(); padded.extend_from_slice(&[0.0;6400]);
            std::fs::write(&temp,audio_chunker::samples_to_wav_bytes(&padded,16000)?).map_err(|e|e.to_string())?;
            let start=std::time::Instant::now();
            let text=match engine {
                "nemotron"=>typr_lib::transcribe_nemotron::transcribe_nemotron(&model,&temp).await?,
                "parakeet"=>typr_lib::transcribe_parakeet::transcribe_parakeet(&model,&temp).await?,
                _=>typr_lib::transcribe_groq::transcribe_groq(&settings.groq_api_key,&temp,"","accurate").await?,
            };
            println!("{engine} {variant}: text={text:?} elapsed_ms={}",start.elapsed().as_millis());
            if normalize(&text)!=normalize(&args[1]) { failures.push(format!("{engine} {variant} did not preserve the expected synthetic utterance")); }
          }
        }
        if failures.is_empty() {Ok(())} else {Err(failures.join("; "))}
    }.await;
    let _=std::fs::remove_file(temp);
    result
}
