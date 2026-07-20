use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::ShellExt;

static LOCAL_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
fn local_client() -> &'static reqwest::Client {
    LOCAL_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            // Safety cap only. The real timeout is set per-request, scaled to audio length
            // (see `post_timeout` below) so a legitimately-slow decode on a battery-throttled
            // GPU is NOT killed and forced down the cold-sidecar fallback.
            .timeout(Duration::from_secs(180))
            .no_proxy() // Disables system proxy detection to prevent WPAD lookup stalls when offline
            .build()
            .unwrap_or_default()
    })
}

/// Estimate recorded audio length (seconds) from a 16 kHz mono s16 WAV's byte length.
/// Used to scale the inference timeout; not exact, but well within the safety margin.
fn estimate_wav_seconds(byte_len: usize) -> f64 {
    // 16000 samples/s * 2 bytes/sample = 32000 B/s of PCM, past the 44-byte header.
    byte_len.saturating_sub(44) as f64 / 32_000.0
}

/// Timeout for a warm-server inference POST, scaled to the clip length. Floor keeps short
/// clips from failing fast into a cold restart on a slow GPU; cap bounds a genuine hang.
fn post_timeout_for(audio_secs: f64) -> Duration {
    Duration::from_secs_f64((audio_secs * 3.0 + 15.0).clamp(20.0, 180.0))
}

/// Non-speech markers whisper.cpp prints for segments it decodes as silence or noise.
///
/// These surfaced once `--no-timestamps` was removed — that flag had been suppressing them —
/// and `[BLANK_AUDIO]` was observed pasted into a real dictation. They are annotations about
/// the audio, never words the user spoke, so they are dropped.
///
/// This is an explicit list rather than a pattern like `[A-Z_]+` on purpose: dictated text
/// can legitimately contain a bracketed capitalised token (`[TODO]`), and a pattern would eat
/// it. Only markers we know the engine emits are removed.
const NON_SPEECH_MARKERS: [&str; 6] = [
    "[BLANK_AUDIO]",
    "[SOUND]",
    "[MUSIC]",
    "[NOISE]",
    "[LAUGHTER]",
    "[APPLAUSE]",
];

/// Spaces whisper.cpp's CLI prints between the `[start --> end]` marker and the segment text.
/// The segment carries its own leading space on top of these, and that one is meaningful —
/// see `normalize_transcript` — so exactly this much padding is removed and no more.
const CLI_MARKER_PAD: usize = 2;

/// Strip a leading `[00:00:09.000 --> 00:00:12.000]` marker, plus the CLI's fixed padding,
/// from one output line. Returns the line unchanged if it doesn't carry a marker.
fn strip_timestamp_marker(line: &str) -> &str {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('[') {
        return line;
    }
    match trimmed.find(']') {
        // Only treat it as a marker if it actually spans a range, so dictated text that
        // legitimately begins with a bracket survives.
        Some(end) if trimmed[..end].contains("-->") => {
            let rest = &trimmed[end + 1..];
            let pad = rest
                .char_indices()
                .take(CLI_MARKER_PAD)
                .take_while(|&(_, c)| c == ' ')
                .count();
            &rest[pad..]
        }
        _ => line,
    }
}

/// Flatten Whisper's per-segment output into the single line the rest of the pipeline expects.
///
/// Whisper emits one segment per line. The warm server returns them clean; the CLI sidecars
/// prefix each with a timestamp marker (we no longer pass `--no-timestamps` — see the note in
/// `whisper_server::ensure_running` for why suppressing timestamps deletes speech).
///
/// `cleanup_text` would collapse these breaks anyway, but the command-bearing path in
/// `recorder::stop_and_transcribe` deliberately skips cleanup, so without this a dictation
/// containing a command would paste with a line break at every segment boundary.
///
/// # Segments are concatenated, not space-joined
///
/// Whisper prefixes a segment with a space when it starts a new word, and omits that space
/// when the segment continues the previous word — a word can straddle a segment boundary.
/// Verified against the warm server's raw output: every ordinary segment came back as
/// `' continuous tape.'`, `' Item 3 is round robin.'`, leading space included.
///
/// So the separator is already in the data. Trimming each segment and re-joining with a space
/// discards that distinction and splits any straddling word — which is how "multilingual" was
/// pasted as "mult ilingual". Concatenating verbatim and collapsing whitespace afterwards
/// preserves both cases.
pub fn normalize_transcript(raw: &str) -> String {
    let mut joined = String::with_capacity(raw.len());
    for line in raw.lines() {
        let mut body = strip_timestamp_marker(line).to_string();
        for marker in NON_SPEECH_MARKERS {
            if body.contains(marker) {
                body = body.replace(marker, "");
            }
        }
        if body.trim().is_empty() {
            continue;
        }
        joined.push_str(&body);
    }

    // Collapse whitespace runs (including the newlines we just dropped) to single spaces.
    let mut out = String::with_capacity(joined.len());
    let mut pending_space = false;
    for ch in joined.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

/// Transcribe with the local Whisper model.
///
/// # The `prompt` argument is accepted and deliberately NOT used
///
/// It carries the dictionary bias terms, and passing them to Whisper as an `initial_prompt`
/// makes the model silently drop speech.
///
/// Measured on `ggml-medium.en-q5_0` with a 77-second dictation containing a seven-item
/// numbered list. With no prompt it transcribed all seven items, twice in a row. With a prompt
/// it returned two of seven and dropped the rest without a trace — and that held for the full
/// dictionary, for three terms, for a prose-style sentence, and for the single word "Tauri".
/// Neither length nor phrasing mattered; only presence.
///
/// The lost words are not mangled, they are absent, and nothing in the output suggests anything
/// is missing. Silent deletion is worse than no bias at all, so no bias is applied here.
///
/// The dictionary still works on this path: `vocab_correct` fixes its terms after
/// transcription rather than biasing before it. Groq is unaffected and still receives the
/// prompt. The parameter stays so the engine signatures match and re-enabling is one line if
/// this is ever fixed upstream.
pub async fn transcribe_local(
    app: &AppHandle,
    model_path: &PathBuf,
    audio_path: &PathBuf,
    _prompt: &str,
) -> Result<String, String> {
    if !model_path.exists() {
        return Err("Whisper model not found. Please download a model first.".to_string());
    }

    let cuda_threads = std::thread::available_parallelism()
        .map(|count| count.get().min(4).to_string())
        .unwrap_or_else(|_| "4".to_string());

    let cpu_threads = std::thread::available_parallelism()
        .map(|count| count.get().min(12).to_string())
        .unwrap_or_else(|_| "12".to_string());

    let resource_path = app.path().resource_dir().unwrap().join("binaries");
    let current_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{};{}", resource_path.to_str().unwrap(), current_path);

    // 1. Try persistent HTTP server first
    println!(
        "[Typr] Attempting persistent local Whisper HTTP server execution with model {:?}",
        model_path
    );
    let http_start = Instant::now();

    // Read audio bytes once
    let file_bytes_result = std::fs::read(audio_path);
    if let Ok(file_bytes) = file_bytes_result {
        // Scale the inference timeout to the clip length so a slow-but-fine decode on a
        // battery-throttled GPU stays on the warm server instead of tripping the fallback.
        let est_audio_secs = estimate_wav_seconds(file_bytes.len());
        let post_timeout = post_timeout_for(est_audio_secs);
        println!(
            "[Typr] Local inference: ~{:.1}s audio; warm-server POST timeout {:.0}s",
            est_audio_secs,
            post_timeout.as_secs_f64()
        );

        // Build the request body helper
        let make_form = |bytes: Vec<u8>| {
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name("audio.wav")
                .mime_str("audio/wav")
                .unwrap();
            // NO PROMPT. See `prompt` on the function signature — passing dictionary hints as
            // an initial prompt makes this model silently skip speech.
            reqwest::multipart::Form::new()
                .part("file", part)
                .text("temperature", "0.0")
                .text("response_format", "json")
        };

        // Guard the hot path on model identity: reuse the warm server only if it is
        // already serving the selected model; otherwise (re)start it first so we never
        // transcribe with a stale model (e.g. right after downloading a new model).
        let intended_key = model_path.to_string_lossy().to_string();
        let current = crate::whisper_server::current_model_key();
        if !crate::whisper_server::warm_server_matches(current.as_deref(), &intended_key) {
            println!(
                "[Typr] Warm server model mismatch (have {:?}, want {}). Ensuring correct model...",
                current, intended_key
            );
            if let Err(e) = crate::whisper_server::ensure_running(app, model_path).await {
                println!(
                    "[Typr] Failed to ensure correct model server: {}. Will still try POST then sidecars...",
                    e
                );
            }
        }

        // POST to the (now correct) warm server.
        let mut http_result = local_client()
            .post("http://127.0.0.1:8080/inference")
            .timeout(post_timeout)
            .multipart(make_form(file_bytes.clone()))
            .send()
            .await;

        // If direct post fails (e.g. connection refused), ensure the server is running, and retry
        if http_result.is_err() {
            println!("[Typr] Direct POST failed. Ensuring persistent server is running...");
            match crate::whisper_server::ensure_running(app, model_path).await {
                Ok(_) => {
                    println!("[Typr] Persistent server ensured healthy. Retrying inference POST...");
                    http_result = local_client()
                        .post("http://127.0.0.1:8080/inference")
                        .timeout(post_timeout)
                        .multipart(make_form(file_bytes))
                        .send()
                        .await;
                }
                Err(e) => {
                    println!("[Typr] Failed to ensure persistent server: {}. Falling back to sidecars...", e);
                }
            }
        }

        // Process the final HTTP result if we have a successful connection
        if let Ok(response) = http_result {
            if response.status().is_success() {
                #[derive(serde::Deserialize)]
                struct InferenceResponse {
                    text: String,
                }
                if let Ok(inf_res) = response.json::<InferenceResponse>().await {
                    let text = normalize_transcript(&inf_res.text);
                    crate::whisper_server::note_activity();
                    let elapsed = http_start.elapsed();
                    println!(
                        "[Typr] Persistent HTTP Whisper completed in {:?} (~{:.2}x realtime, {:.1}s audio). Output: {}",
                        elapsed,
                        elapsed.as_secs_f64() / est_audio_secs.max(0.1),
                        est_audio_secs,
                        text
                    );
                    return Ok(text);
                }
            } else {
                println!("[Typr] HTTP server returned error: {}. Falling back to sidecars...", response.status());
            }
        } else {
            println!("[Typr] Persistent HTTP server failed. Falling back to sidecars...");
        }
    } else {
        println!("[Typr] Failed to read audio file. Falling back to sidecars...");
    }

    // 2. Try GPU (CUDA) execution as fallback
    println!(
        "[Typr] Attempting whisper.cpp GPU (CUDA) execution with model {:?} using {} threads",
        model_path, cuda_threads
    );
    let started_gpu = Instant::now();

    let cuda_cmd_args = vec![
        "-m".to_string(),
        model_path.to_str().unwrap().to_string(),
        "-f".to_string(),
        audio_path.to_str().unwrap().to_string(),
        // No `--no-timestamps`: see the note in `whisper_server::ensure_running`. It makes the
        // decoder drop speech. The CLI prefixes each line with a `[start --> end]` marker as a
        // result, which `normalize_transcript` strips.
        "-t".to_string(),
        cuda_threads,
        // Beam search, not greedy: see the note in `whisper_server::ensure_running`.
        // `-bs 1` silently drops enumerated speech.
        "-bs".to_string(),
        "5".to_string(),
        "-mc".to_string(),
        "0".to_string(),
        "-nf".to_string(),
        "-l".to_string(),
        "en".to_string(),
    ];

    // No --prompt: see the note on the `prompt` parameter. The sidecars run the same model
    // as the server and skip speech the same way.

    let gpu_result = app
        .shell()
        .sidecar("whisper-cpp-cuda")
        .map_err(|e| format!("Failed to create sidecar command: {}", e))?
        .env("PATH", &new_path)
        .args(cuda_cmd_args)
        .output()
        .await;

    match gpu_result {
        Ok(output) if output.status.success() => {
            let text = normalize_transcript(&String::from_utf8_lossy(&output.stdout));
            println!(
                "[Typr] GPU (CUDA) Whisper completed in {:?}. Output: {}",
                started_gpu.elapsed(),
                text
            );
            return Ok(text);
        }
        other => {
            let error_details = match &other {
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    format!("Exit status: {:?}, Stderr: {}", output.status, stderr)
                }
                Err(e) => e.to_string(),
            };
            println!(
                "[Typr] GPU (CUDA) execution failed or not available. Error: {}. Falling back to CPU...",
                error_details
            );
        }
    }

    // 2. CPU Fallback Path
    println!(
        "[Typr] Running CPU fallback sidecar with model {:?} using {} threads",
        model_path, cpu_threads
    );
    let started_cpu = Instant::now();

    let cpu_cmd_args = vec![
        "-m".to_string(),
        model_path.to_str().unwrap().to_string(),
        "-f".to_string(),
        audio_path.to_str().unwrap().to_string(),
        // No `--no-timestamps`: see the note in `whisper_server::ensure_running`. It makes the
        // decoder drop speech. The CLI prefixes each line with a `[start --> end]` marker as a
        // result, which `normalize_transcript` strips.
        "-t".to_string(),
        cpu_threads,
        // Beam search, not greedy: see the note in `whisper_server::ensure_running`.
        "-bs".to_string(),
        "5".to_string(),
        "-mc".to_string(),
        "0".to_string(),
        "-nf".to_string(),
        "-l".to_string(),
        "en".to_string(),
    ];

    // No --prompt: see the note on the `prompt` parameter.

    let cpu_output = app
        .shell()
        .sidecar("whisper-cpp")
        .map_err(|e| format!("Failed to create sidecar command: {}", e))?
        .env("PATH", &new_path)
        .args(cpu_cmd_args)
        .output()
        .await;

    match cpu_output {
        Ok(output) if output.status.success() => {
            let text = normalize_transcript(&String::from_utf8_lossy(&output.stdout));
            println!(
                "[Typr] CPU Fallback Whisper completed in {:?}. Output: {}",
                started_cpu.elapsed(),
                text
            );
            Ok(text)
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("whisper.cpp CPU fallback failed with exit status: {:?}. Stderr: {}", output.status, stderr))
        }
        Err(e) => {
            Err(format!("Failed to run whisper.cpp CPU fallback: {}", e))
        }
    }
}

pub fn model_filename(model_size: &str) -> String {
    format!("ggml-{}.bin", model_size)
}

pub fn model_download_url(model_size: &str) -> String {
    format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{}.bin",
        model_size
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_filename() {
        assert_eq!(model_filename("small"), "ggml-small.bin");
        assert_eq!(model_filename("medium"), "ggml-medium.bin");
        // English quantized IDs (the current defaults) must map to the real HF filenames.
        assert_eq!(model_filename("small.en-q5_1"), "ggml-small.en-q5_1.bin");
        assert_eq!(model_filename("medium.en-q5_0"), "ggml-medium.en-q5_0.bin");
    }

    #[test]
    fn test_model_download_url() {
        assert_eq!(
            model_download_url("small"),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin"
        );
        assert_eq!(
            model_download_url("medium.en-q5_0"),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en-q5_0.bin"
        );
    }

    #[test]
    fn test_normalize_transcript_strips_cli_timestamps() {
        // The CLI shape once `--no-timestamps` is gone.
        let raw = "\n[00:00:00.000 --> 00:00:04.000]   This is a test.\n\
                   [00:00:04.000 --> 00:00:09.000]   Item 1 is first come first serve.\n";
        assert_eq!(
            normalize_transcript(raw),
            "This is a test. Item 1 is first come first serve."
        );
    }

    #[test]
    fn test_normalize_transcript_flattens_server_segments() {
        // The warm server returns clean text, one segment per line, no markers. Those breaks
        // must still collapse: the command-bypass path skips `cleanup_text`, so a stray
        // newline here would land in the pasted output.
        let raw = " This is a transcription accuracy test\n recorded in a single take.\n Item 1.\n";
        assert_eq!(
            normalize_transcript(raw),
            "This is a transcription accuracy test recorded in a single take. Item 1."
        );
    }

    #[test]
    fn test_normalize_transcript_preserves_dictated_brackets() {
        // A line that genuinely starts with a bracket is not a timestamp marker and must
        // survive intact — only `[start --> end]` is stripped.
        assert_eq!(normalize_transcript("[TODO] fix this"), "[TODO] fix this");
        assert_eq!(normalize_transcript("[draft] and [final]"), "[draft] and [final]");
    }

    #[test]
    fn test_normalize_transcript_keeps_word_split_across_segments() {
        // A word can straddle a segment boundary. Whisper signals it by omitting the leading
        // space on the continuation, so the two halves must be concatenated, not space-joined.
        // Space-joining here is what pasted "multilingual" as "mult ilingual".
        let server = " This is to test the medium mult\nilingual model.";
        assert_eq!(
            normalize_transcript(server),
            "This is to test the medium multilingual model."
        );

        // Same case through the CLI, where the marker and its two-space pad precede the
        // segment's own (here absent) leading space.
        let cli = "[00:00:00.000 --> 00:00:03.000]   the medium mult\n\
                   [00:00:03.000 --> 00:00:04.200]  ilingual model.";
        assert_eq!(normalize_transcript(cli), "the medium multilingual model.");
    }

    #[test]
    fn test_normalize_transcript_drops_non_speech_markers() {
        // Observed in a real dictation once `--no-timestamps` was removed: the marker was
        // pasted verbatim at the end of the user's sentence.
        assert_eq!(
            normalize_transcript(" I want to know the status [BLANK_AUDIO]"),
            "I want to know the status"
        );
        // A marker occupying its whole segment leaves nothing behind — no stray space.
        assert_eq!(
            normalize_transcript(" first part.\n [BLANK_AUDIO]\n second part."),
            "first part. second part."
        );
        // Other engine annotations go too.
        assert_eq!(normalize_transcript(" hello [MUSIC] world"), "hello world");
        // But a bracketed word the user actually dictated must survive.
        assert_eq!(normalize_transcript("[TODO] fix this"), "[TODO] fix this");
    }

    #[test]
    fn test_normalize_transcript_edges() {
        assert_eq!(normalize_transcript(""), "");
        assert_eq!(normalize_transcript("   \n\n  \n"), "");
        // Blank segments are dropped rather than becoming double spaces. Real segments carry
        // their own leading space, which is what separates them.
        assert_eq!(normalize_transcript(" one\n\n\n two"), "one two");
        // Without that leading space the two are one word, by the same rule that keeps
        // "mult" + "ilingual" together.
        assert_eq!(normalize_transcript(" one\ntwo"), "onetwo");
        // A marker with no text after it contributes nothing.
        assert_eq!(
            normalize_transcript("[00:00:00.000 --> 00:00:04.000]\nreal text"),
            "real text"
        );
    }

    #[test]
    fn test_estimate_wav_seconds() {
        // 32000 B/s of PCM after a 44-byte header.
        assert_eq!(estimate_wav_seconds(44), 0.0);
        assert_eq!(estimate_wav_seconds(44 + 32_000), 1.0);
        assert_eq!(estimate_wav_seconds(44 + 320_000), 10.0);
        // Undersized/garbage input never underflows.
        assert_eq!(estimate_wav_seconds(0), 0.0);
    }

    #[test]
    fn test_post_timeout_for() {
        // Short clips get the floor, not a fail-fast 5s.
        assert_eq!(post_timeout_for(0.0), Duration::from_secs(20));
        assert_eq!(post_timeout_for(1.0), Duration::from_secs(20));
        // Mid-length scales up (5s -> 30s).
        assert_eq!(post_timeout_for(5.0), Duration::from_secs(30));
        // Long clips scale further (34s -> 117s), well past a slow decode.
        assert_eq!(post_timeout_for(34.0), Duration::from_secs(117));
        // Cap bounds a genuine hang.
        assert_eq!(post_timeout_for(1000.0), Duration::from_secs(180));
    }
}
