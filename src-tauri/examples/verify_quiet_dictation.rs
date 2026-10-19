//! Reference-audio check; never records a microphone or reads a user document.
//! Set TYPR_LIVE_ASR_TEST=1 to additionally send two repository clips to Groq.
use std::{path::PathBuf, time::Instant};
use typr_lib::{audio, audio_chunker, settings::Settings, speech_detection};

#[tokio::main]
async fn main() -> Result<(), String> {
    let app_dir = dirs::config_dir().ok_or("No config directory")?.join("com.typr.app");
    let clip = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/test_clips/clip1_5s.wav");
    let (source, rate) = audio_chunker::read_wav_samples(&clip)?;
    // Optional local-only fixtures, for example synthesized greetings. These
    // files are checked by VAD only and are never sent to a service.
    for fixture in std::env::args_os().skip(1) {
        let (samples, rate) = audio_chunker::read_wav_samples(&PathBuf::from(fixture))?;
        assert!(speech_detection::contains_speech(&samples, rate)?, "spoken fixture rejected");
        println!("PASS: additional spoken fixture retained");
    }
    let temporary = std::env::temp_dir().join(format!("typr-quiet-check-{}.wav", std::process::id()));
    let result = async {
        let mut previous = std::collections::BTreeMap::<String, String>::new();
        for scale in [1.0, 0.025, 0.005] {
            let raw: Vec<f32> = source.iter().map(|s| s * scale).collect();
            let start = Instant::now();
            assert!(speech_detection::contains_speech(&raw, rate)?);
            println!("Speech gate scale={scale} elapsed_ms={}", start.elapsed().as_millis());
            let mut samples = audio::resample(&raw, rate, 16000);
            audio::process_speech_audio_chain(&mut samples, 16000, audio::FIXED_INPUT_GAIN_DB);
            samples.extend_from_slice(&[0.0; 6400]);
            std::fs::write(&temporary, audio_chunker::samples_to_wav_bytes(&samples, 16000)?).map_err(|e|e.to_string())?;
            for engine in ["nemotron", "parakeet", "cloud"] {
                let model = app_dir.join(if engine == "nemotron" {"nemotron-3.5-asr-streaming-0.6b-int8"} else {"parakeet-tdt-0.6b-v3-int8"});
                if engine == "cloud" && std::env::var("TYPR_LIVE_ASR_TEST").as_deref() != Ok("1") {continue;}
                if engine != "cloud" && !model.is_dir() { println!("SKIP {engine}: model not installed"); continue; }
                if scale == 1.0 {
                    if engine == "nemotron" {typr_lib::transcribe_nemotron::prewarm(&model)?;}
                    if engine == "parakeet" {typr_lib::transcribe_parakeet::prewarm(&model)?;}
                }
                let start = Instant::now();
                let text = match engine {
                    "nemotron" => typr_lib::transcribe_nemotron::transcribe_nemotron(&model, &temporary).await?,
                    "parakeet" => typr_lib::transcribe_parakeet::transcribe_parakeet(&model, &temporary).await?,
                    _ => {
                        let settings = Settings::load(&app_dir);
                        typr_lib::transcribe_groq::transcribe_groq(&settings.groq_api_key, &temporary, "", "accurate").await?
                    }
                };
                println!("{engine} scale={scale} elapsed_ms={} text={text:?}", start.elapsed().as_millis());
                if let Some(original) = previous.get(engine) {
                    assert_eq!(text, *original, "{engine} quiet transcript regressed against normal reference");
                } else {previous.insert(engine.into(), text);}
            }
        }
        assert!(!speech_detection::contains_speech(&vec![0.0; 96000], 16000)?);
        println!("PASS: quiet speech, engine reference comparison, silent follow-up capture");
        Ok(())
    }.await;
    let _ = std::fs::remove_file(temporary);
    result
}
