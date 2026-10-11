use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{WavSpec, WavWriter};
use rustfft::{Fft, FftPlanner, num_complex::Complex};
use std::sync::Arc as StdArc;
use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use tauri::AppHandle;

static CLIPPING_TOAST_NOTIFIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static QUIET_TOAST_NOTIFIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Debug, Clone, serde::Serialize)]
pub struct MicDevice {
    pub name: String,
    pub is_default: bool,
}

pub fn list_microphones() -> Vec<MicDevice> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();

    let mut devices = Vec::new();
    if let Ok(input_devices) = host.input_devices() {
        for device in input_devices {
            if let Ok(name) = device.name() {
                devices.push(MicDevice {
                    is_default: name == default_name,
                    name,
                });
            }
        }
    }
    devices
}

#[derive(Debug, PartialEq)]
pub struct MicResolution {
    pub target: String,
    pub fell_back: bool,
}

/// Decide which device to actually open.
/// - "default"              -> the current default device
/// - a named device present -> itself
/// - a named device absent  -> fall back to default (fell_back = true)
/// - nothing usable         -> Err
pub fn resolve_mic(setting: &str, available: &[String], default: Option<&str>) -> Result<MicResolution, String> {
    if setting == "default" {
        return match default {
            Some(d) => Ok(MicResolution { target: d.to_string(), fell_back: false }),
            None => Err("No default input device found".to_string()),
        };
    }
    if available.iter().any(|n| n == setting) {
        return Ok(MicResolution { target: setting.to_string(), fell_back: false });
    }
    match default {
        Some(d) => Ok(MicResolution { target: d.to_string(), fell_back: true }),
        None => Err(format!("Microphone '{}' not found and no default input device available", setting)),
    }
}

/// Whether the warm (paused) stream can be reused for `requested` without rebuilding —
/// keeps device enumeration off the record hot path.
pub fn can_reuse_stream(stream_present: bool, errored: bool, active_setting: Option<&str>, requested: &str) -> bool {
    stream_present && !errored && active_setting == Some(requested)
}

#[derive(Debug, Clone)]
pub struct MicStartInfo {
    pub active_device: String,
    pub fell_back: bool,
    pub changed: bool,
}

struct SendStream(#[allow(dead_code)] cpal::Stream);
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}

pub struct AudioRecorder {
    samples: Arc<Mutex<Vec<f32>>>,
    stream: Option<SendStream>,
    // The mic setting (e.g. "default" or a device name) the current stream was built for.
    // Reuse keys on this so the hot path skips device enumeration entirely.
    active_setting: Option<String>,
    active_resolved_name: Option<String>,
    stream_errored: Arc<std::sync::atomic::AtomicBool>,
    source_sample_rate: u32,
    source_channels: u16,
    amplitude_ring: Arc<Mutex<Vec<f32>>>,
    amplitude_index: Arc<Mutex<usize>>,
    fft: StdArc<dyn Fft<f32>>,
    fft_buffer: Arc<Mutex<Vec<Complex<f32>>>>,
    frequency_bands: Arc<Mutex<Vec<f32>>>,
    fft_callback_divider: Arc<Mutex<u8>>,
    /// Slowly-adapting AGC peak: a stable normalization reference across FFT blocks so
    /// the bar shape doesn't re-scale to each block's own max (the shimmer source).
    agc_max: Arc<Mutex<f32>>,
    /// Linear input-gain factor shown on the live amplitude meter, so the bar reflects
    /// what the transcription chain will actually hear (set from settings before start).
    meter_gain: Arc<Mutex<f32>>,
}

/// Asymmetric temporal EMA: fast attack (rising toward a higher target), slow release
/// (falling toward a lower target). Result clamped to [0.0, 1.0].
pub fn smooth_band(prev: f32, target: f32, attack: f32, release: f32) -> f32 {
    let coeff = if target > prev { attack } else { release };
    (prev + (target - prev) * coeff).clamp(0.0, 1.0)
}

/// Running peak for AGC. Rising toward a louder block is now *eased* by `rise` (instead
/// of snapping), so a single transient — a cough, a door slam, one over-loud word — can
/// only nudge the reference a little; a genuinely sustained louder passage still climbs
/// over a few blocks. Quieter blocks decay gently toward the current block max. Never
/// falls below a small floor so we never divide by ~zero.
pub fn agc_update(prev_max: f32, block_max: f32, decay: f32, rise: f32) -> f32 {
    const FLOOR: f32 = 0.0001;
    let next = if block_max > prev_max {
        prev_max + (block_max - prev_max) * rise
    } else {
        (prev_max * decay).max(block_max)
    };
    next.max(FLOOR)
}

impl AudioRecorder {
    pub fn new() -> Self {
        let fft_size = 4096;
        let mut fft_buffer = Vec::with_capacity(fft_size);
        fft_buffer.resize(fft_size, Complex::new(0.0, 0.0));
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(fft_size);
        
        Self {
            samples: Arc::new(Mutex::new(Vec::new())),
            stream: None,
            active_setting: None,
            active_resolved_name: None,
            stream_errored: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            source_sample_rate: 48000,
            source_channels: 1,
            amplitude_ring: Arc::new(Mutex::new(vec![0.0; 64])),
            amplitude_index: Arc::new(Mutex::new(0)),
            fft,
            fft_buffer: Arc::new(Mutex::new(fft_buffer)),
            frequency_bands: Arc::new(Mutex::new(vec![0.0; 16])),
            fft_callback_divider: Arc::new(Mutex::new(0)),
            agc_max: Arc::new(Mutex::new(0.0001)),
            meter_gain: Arc::new(Mutex::new(1.0)),
        }
    }

    /// Set the live meter's gain factor from the user's input-gain setting (linear).
    pub fn set_meter_gain(&self, gain_db: f32) {
        *self.meter_gain.lock().unwrap() = 10f32.powf(gain_db / 20.0);
    }

    pub fn get_amplitude_ring(&self) -> Vec<f32> {
        let ring = self.amplitude_ring.lock().unwrap();
        let idx = *self.amplitude_index.lock().unwrap();
        let size = ring.len();
        let mut result = Vec::with_capacity(size);
        for i in 0..size {
            let pos = (idx + i + 1) % size;
            result.push(ring[pos]);
        }
        result
    }
    
    pub fn get_frequency_bands(&self) -> Vec<f32> {
        self.frequency_bands.lock().unwrap().clone()
    }

    /// Slice new unread raw PCM float samples since `start` index, aligning to whole frames
    /// if multi-channel. Returns `(slice, new_aligned_end, source_sample_rate, source_channels)`.
    pub fn get_raw_samples_from(&self, start: usize) -> (Vec<f32>, usize, u32, u16) {
        let buf = self.samples.lock().unwrap();
        let current_len = buf.len();
        let frame_size = self.source_channels.max(1) as usize;
        let aligned_len = if frame_size > 1 {
            current_len - (current_len % frame_size)
        } else {
            current_len
        };
        if aligned_len <= start {
            return (Vec::new(), start, self.source_sample_rate, self.source_channels);
        }
        let slice = buf[start..aligned_len].to_vec();
        (slice, aligned_len, self.source_sample_rate, self.source_channels)
    }

    /// Release the raw capture buffer. Called by the recorder after the engine is done with it.
    pub fn clear_samples(&self) {
        let mut samples = self.samples.lock().unwrap();
        samples.clear();
        // A long dictation must not retain its peak allocation for the entire
        // tray session. Keep a small reusable buffer for the next recording.
        let second = self.source_sample_rate as usize * self.source_channels as usize;
        if samples.capacity() > second * 10 { samples.shrink_to(second * 2); }
    }

    pub fn source_sample_rate(&self) -> u32 {
        self.source_sample_rate
    }

    #[cfg(test)]
    pub fn push_raw_samples_for_test(&self, data: &[f32]) {
        self.samples.lock().unwrap().extend_from_slice(data);
    }

    #[cfg(test)]
    pub fn set_source_format_for_test(&mut self, sample_rate: u32, channels: u16) {
        self.source_sample_rate = sample_rate;
        self.source_channels = channels;
    }

    pub fn ensure_initialized(&mut self, mic_name: &str) -> Result<MicStartInfo, String> {
        // Fast path: reuse the warm stream for the same setting without touching cpal
        // enumeration (that enumeration was the ~1-2s dead window on the record path).
        let errored = self.stream_errored.load(std::sync::atomic::Ordering::Relaxed);
        if can_reuse_stream(self.stream.is_some(), errored, self.active_setting.as_deref(), mic_name) {
            let active = self.active_resolved_name.clone().unwrap_or_else(|| mic_name.to_string());
            return Ok(MicStartInfo { active_device: active, fell_back: false, changed: false });
        }

        // Rebuild path: enumerate live devices + default to resolve the real target.
        let host = cpal::default_host();
        let default_name = host.default_input_device().and_then(|d| d.name().ok());
        let available: Vec<String> = host
            .input_devices()
            .map(|it| it.filter_map(|d| d.name().ok()).collect())
            .unwrap_or_default();
        let res = resolve_mic(mic_name, &available, default_name.as_deref())?;

        // Open the resolved device. Prefer the default object when the setting is
        // "default" to avoid a same-name ambiguity between two devices.
        let device = if mic_name == "default" {
            host.default_input_device()
                .ok_or("No default input device found")?
        } else {
            host.input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().map(|n| n == res.target).unwrap_or(false))
                .ok_or(format!("Microphone '{}' not found", res.target))?
        };

        let default_config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get default input config: {}", e))?;

        let sample_rate = default_config.sample_rate().0;
        let channels = default_config.channels();

        println!("[Typr] Mic config: {}Hz, {} channels", sample_rate, channels);

        self.source_sample_rate = sample_rate;
        self.source_channels = channels;

        let config = cpal::StreamConfig {
            channels,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let samples = self.samples.clone();
        let amplitude_ring = self.amplitude_ring.clone();
        let amplitude_index = self.amplitude_index.clone();
        let fft = self.fft.clone();
        let fft_buffer = self.fft_buffer.clone();
        let frequency_bands = self.frequency_bands.clone();
        let fft_callback_divider = self.fft_callback_divider.clone();
        let agc_max = self.agc_max.clone();
        let meter_gain = self.meter_gain.clone();
        let stream_errored = self.stream_errored.clone();

        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut buf = samples.lock().unwrap();
                    buf.extend_from_slice(data);

                    let rms = (data.iter().map(|&x| x * x).sum::<f32>() / data.len() as f32).sqrt();
                    let gain = *meter_gain.lock().unwrap();
                    let normalized_amp = (rms * gain * 8.0).min(1.0);

                    let mut ring = amplitude_ring.lock().unwrap();
                    let mut idx = amplitude_index.lock().unwrap();
                    ring[*idx] = normalized_amp;
                    *idx = (*idx + 1) % ring.len();
                    
                    let fft_size = 4096;
                    let buf_len = buf.len();
                    let should_update_fft = {
                        let mut divider = fft_callback_divider.lock().unwrap();
                        *divider = (*divider + 1) % 4;
                        *divider == 0
                    };

                    if should_update_fft && buf_len >= fft_size {
                        let mut buffer = fft_buffer.lock().unwrap();
                        let window_start = buf_len - fft_size;
                        for i in 0..fft_size {
                            let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (fft_size - 1) as f32).cos());
                            buffer[i] = Complex::new(buf[window_start + i] * window, 0.0);
                        }
                        
                        fft.process(&mut buffer);
                        
                        let num_bands = 16;
                        let mut bands = frequency_bands.lock().unwrap();
                        
                        let min_freq = 80.0f32;
                        let max_freq = 500.0f32;
                        
                        let mut block_energy = 0.0f32;
                        for i in 0..fft_size {
                            let val = buf[window_start + i];
                            block_energy += val * val;
                        }
                        let block_rms = (block_energy / fft_size as f32).sqrt();
                        
                        let mut amplitudes = vec![0.0; num_bands];
                        
                        for band in 0..num_bands {
                            let band_min_freq = min_freq * (max_freq / min_freq).powf(band as f32 / num_bands as f32);
                            let band_max_freq = min_freq * (max_freq / min_freq).powf((band + 1) as f32 / num_bands as f32);
                            
                            let start_bin = (band_min_freq * fft_size as f32 / sample_rate as f32).round() as usize;
                            let end_bin = (band_max_freq * fft_size as f32 / sample_rate as f32).round() as usize;
                            let end_bin = end_bin.max(start_bin + 1).min(fft_size / 2);
                            
                            let mut energy = 0.0f32;
                            for bin in start_bin..end_bin {
                                energy += buffer[bin].norm_sqr();
                            }
                            
                            amplitudes[band] = energy.sqrt();
                        }
                        
                        // Spatial smoothing to make the bars move together seamlessly.
                        let mut smoothed_amplitudes = vec![0.0; num_bands];
                        let mut block_max = 0.0001f32;

                        for i in 0..num_bands {
                            let mut val = amplitudes[i] * 0.4;
                            if i > 0 { val += amplitudes[i - 1] * 0.2; }
                            if i > 1 { val += amplitudes[i - 2] * 0.1; }
                            if i + 1 < num_bands { val += amplitudes[i + 1] * 0.2; }
                            if i + 2 < num_bands { val += amplitudes[i + 2] * 0.1; }

                            smoothed_amplitudes[i] = val;
                            if val > block_max {
                                block_max = val;
                            }
                        }

                        // Creamy temporal smoothing: fast attack, slow release. The AGC
                        // peak adapts slowly so the bar shape stays stable across blocks
                        // instead of re-normalizing to each block's own max (shimmer), and
                        // silence fades out through the release instead of a hard-zero blink.
                        const ATTACK: f32 = 0.5;
                        const RELEASE: f32 = 0.22;
                        const DECAY: f32 = 0.995;
                        // Eased AGC rise: a transient spike only nudges the reference, so it
                        // can't pin normalization high and flatten everything spoken after it.
                        const RISE: f32 = 0.3;

                        let agc = {
                            let mut m = agc_max.lock().unwrap();
                            *m = agc_update(*m, block_max, DECAY, RISE);
                            *m
                        };

                        let noise_gate = 0.003;
                        let is_speaking = block_rms > noise_gate;

                        for band in 0..num_bands {
                            let target = if is_speaking {
                                let normalized = smoothed_amplitudes[band] / agc;
                                let shape = normalized.powi(2); // rounded peak, not harsh spikes
                                let volume_factor = ((block_rms - noise_gate) * 18.0).min(1.0);
                                let pitch_height_boost = 1.0 + (band as f32 * 0.05);
                                (shape * volume_factor * pitch_height_boost).min(1.0)
                            } else {
                                0.0
                            };
                            bands[band] = smooth_band(bands[band], target, ATTACK, RELEASE);
                        }
                    }
                },
                move |err| {
                    eprintln!("[Typr] Audio stream error: {}", err);
                    stream_errored.store(true, std::sync::atomic::Ordering::Relaxed);
                },
                None,
            )
            .map_err(|e| e.to_string())?;

        // Leave the freshly built stream paused (idle); start()/the warm-up will play it.
        let _ = stream.pause();
        self.stream = Some(SendStream(stream));
        self.active_setting = Some(mic_name.to_string());
        self.active_resolved_name = Some(res.target.clone());
        self.stream_errored.store(false, std::sync::atomic::Ordering::Relaxed);
        println!("[Typr] Audio stream (re)built (paused) for '{}' (device '{}')", mic_name, res.target);
        Ok(MicStartInfo { active_device: res.target, fell_back: res.fell_back, changed: true })
    }

    /// Play the (pre-built, paused) stream to warm the device without recording — used by
    /// the one-time startup warm-up. Callbacks that fire during warm-up are discarded by
    /// `device_pause_idle`/`start`.
    pub fn device_play(&self) {
        if let Some(ref s) = self.stream {
            let _ = s.0.play();
        }
    }

    /// Settle the warmed stream back to idle (paused) and drop any warm-up samples.
    pub fn device_pause_idle(&self) {
        if let Some(ref s) = self.stream {
            let _ = s.0.pause();
        }
        self.samples.lock().unwrap().clear();
    }

    pub fn start(&mut self, mic_name: &str) -> Result<MicStartInfo, String> {
        // Reuse the warm stream (fast, no enumeration) or rebuild if the mic changed.
        let info = self.ensure_initialized(mic_name)?;

        self.samples.lock().unwrap().clear();
        {
            let mut ring = self.amplitude_ring.lock().unwrap();
            for v in ring.iter_mut() {
                *v = 0.0;
            }
        }
        *self.amplitude_index.lock().unwrap() = 0;

        // Start capture on the (already-activated) device — a fast start after warm-up.
        if let Some(ref s) = self.stream {
            s.0.play().map_err(|e| e.to_string())?;
        }
        println!("[Typr] Audio recording started");
        Ok(info)
    }

    pub fn stop_and_save(
        &mut self,
        output_path: &PathBuf,
        app: Option<&AppHandle>,
    ) -> Result<(PathBuf, f32), String> {
        let duration = self.stop_capture()?;
        self.save_capture(output_path, app)?;
        Ok((output_path.clone(), duration))
    }

    /// Stop and validate without preparing a second copy of audio that a live
    /// recognizer has already consumed. Keep raw samples for its final drain.
    pub fn stop_capture(&mut self) -> Result<f32, String> {
        // Pause the stream (mic off between records); the device stays activated so the
        // next start() is a fast play with no dropped audio.
        if let Some(ref s) = self.stream {
            let _ = s.0.pause();
        }
        println!("[Typr] Audio recording stopped (mic paused)");

        let samples = self.samples.lock().unwrap();
        if samples.is_empty() {
            return Err("No audio captured".to_string());
        }
        
        let duration_secs = samples.len() as f32 / self.source_channels as f32 / self.source_sample_rate as f32;
        if duration_secs < 0.4 {
            return Err("Audio too short".to_string());
        }
        
        // The silence gate judges the signal AFTER the fixed native input gain, not the
        // raw mic level — otherwise quiet-but-intended speech is rejected as "silent"
        // before the gain ever applies. Two-condition gate (FluidVoice-style): mean AND
        // max-frame must both be low, so a softly-spoken sentence inside a mostly-silent
        // clip survives on its 20 ms frames even when the long-clip mean falls below
        // the threshold.
        let gain_factor = 10f32.powf(FIXED_INPUT_GAIN_DB / 20.0);
        // Samples are interleaved: a 20 ms frame includes all device channels.
        if is_silent_capture(&samples,
            self.source_sample_rate * self.source_channels as u32, gain_factor) {
            return Err("Audio was silent".to_string());
        }

        println!("[Typr] Captured {} raw samples", samples.len());
        Ok(duration_secs)
    }

    /// Prepare the stopped capture only when batch recognition or diagnostics
    /// needs a WAV. Must be called after stop_capture, before clear_samples.
    pub fn save_capture(
        &self,
        output_path: &PathBuf,
        app: Option<&AppHandle>,
    ) -> Result<(), String> {
        let samples = self.samples.lock().unwrap();
        if samples.is_empty() {
            return Err("No audio captured".to_string());
        }

        let mono: Vec<f32> = if self.source_channels > 1 {
            samples
                .chunks(self.source_channels as usize)
                .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
                .collect()
        } else {
            samples.clone()
        };

        let mut resampled = resample(&mono, self.source_sample_rate, 16000);
        let diagnostics = process_speech_audio_chain(&mut resampled, 16000, FIXED_INPUT_GAIN_DB);
        // Measured BEFORE the padding below: how loud is the last 150 ms of what was actually
        // captured, relative to the clip's speech level? ~1.0x = the capture ended mid-speech
        // (cut off at the source); ~0.1x or lower = the last word was captured in full.
        let tail_150 = tail_rms(&resampled, 16000, 150);
        let tail_ratio = if diagnostics.post_rms > 1e-6 { tail_150 / diagnostics.post_rms } else { 0.0 };
        // Tail comfort silence padding: append 400ms (6400 samples @ 16kHz) of silence
        // so that conformer lookahead, RNNT joiner, and Whisper attention have acoustic
        // closure and future context frames. Prevents dropping the last 2-3 words.
        let tail_silence_samples = (16000.0 * 0.400) as usize;
        resampled.extend(std::iter::repeat(0.0f32).take(tail_silence_samples));
        println!(
            "[Typr] Audio preprocessed: resampled to {} samples @ 16kHz, pre-p95: {:.4}, post-p95: {:.4}, clips: {}, tail150: {:.4} ({:.2}x speech level)",
            resampled.len(), diagnostics.p95_rms, diagnostics.post_rms, diagnostics.clip_count, tail_150, tail_ratio
        );

        if diagnostics.clipping_detected && !CLIPPING_TOAST_NOTIFIED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            if let Some(app) = app {
                crate::recorder::notify_user(
                    app,
                    "Microphone audio is clipping — try speaking further from the mic.",
                );
            }
        }
        if diagnostics.is_quiet && !QUIET_TOAST_NOTIFIED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            if let Some(app) = app {
                crate::recorder::notify_user(
                    app,
                    "Microphone audio is very quiet — try speaking closer to the mic.",
                );
            }
        }

        let spec = WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer = WavWriter::create(output_path, spec).map_err(|e| e.to_string())?;
        for &sample in resampled.iter() {
            let amplitude = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(amplitude).map_err(|e| e.to_string())?;
        }
        writer.finalize().map_err(|e| e.to_string())?;

        drop(samples);
        // NOTE: the raw sample buffer is intentionally NOT cleared here. The Nemotron live
        // session drains its final unread samples from this buffer in `finish()`, which runs
        // AFTER this method; clearing here turned that drain into a silent no-op and dropped
        // up to one 150 ms poll interval of audio. `start()` clears the buffer at the beginning
        // of the next recording, and `Recorder::stop_and_transcribe` calls `clear_samples()`
        // once the engine has finished with it.

        println!("[Typr] WAV saved to {:?}", output_path);
        Ok(())
    }

    /// Live dictation still reports input problems without running the whole
    /// batch filter. These levels are measured on the microphone signal.
    pub fn notify_capture_levels(&self, app: &AppHandle) {
        let samples = self.samples.lock().unwrap();
        let clipped = samples.iter().any(|s| s.abs() >= 0.999);
        let quiet = measure_p95_frame_rms(&samples,
            self.source_sample_rate * self.source_channels as u32)
            * 10f32.powf(FIXED_INPUT_GAIN_DB / 20.0) < 0.010;
        if clipped && !CLIPPING_TOAST_NOTIFIED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            crate::recorder::notify_user(app,
                "Microphone audio is clipping — try speaking further from the mic.");
        }
        if quiet && !QUIET_TOAST_NOTIFIED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            crate::recorder::notify_user(app,
                "Microphone audio is very quiet — try speaking closer to the mic.");
        }
    }

    /// Opt-in review artifact: exact interleaved device samples, before gain,
    /// filtering, or resampling. Never overwrite another recording.
    pub fn save_review_capture(&self, output_path: &PathBuf) -> Result<(), String> {
        self.review_capture().save(output_path)
    }

    /// Take a short in-memory snapshot so disk I/O never holds the live worker's
    /// sample lock or delays paste. Only called when review recording is enabled.
    pub fn review_capture(&self) -> ReviewCapture {
        ReviewCapture {
            channels: self.source_channels,
            sample_rate: self.source_sample_rate,
            samples: self.samples.lock().unwrap().clone(),
        }
    }
}

pub struct ReviewCapture {
    channels: u16,
    sample_rate: u32,
    samples: Vec<f32>,
}

impl ReviewCapture {
    pub fn save(&self, output_path: &PathBuf) -> Result<(), String> {
        let file = std::fs::OpenOptions::new().write(true).create_new(true)
            .open(output_path).map_err(|e| e.to_string())?;
        let result = (|| {
            // WavWriter::new does not add buffering. A bare File turned each
            // four-byte sample into a Windows write and delayed long dictations
            // by seconds, even when live decoding was already complete.
            let buffered = std::io::BufWriter::with_capacity(256 * 1024, file);
            let mut writer = WavWriter::new(buffered, WavSpec {
                channels: self.channels,
                sample_rate: self.sample_rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            }).map_err(|e| e.to_string())?;
            for &sample in self.samples.iter() {
                writer.write_sample(sample).map_err(|e| e.to_string())?;
            }
            writer.finalize().map_err(|e| e.to_string())
        })();
        if result.is_err() { let _ = std::fs::remove_file(output_path); }
        result
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreprocessDiagnostics {
    pub clipping_detected: bool,
    pub clip_count: usize,
    pub p95_rms: f32,
    pub post_rms: f32,
    pub is_quiet: bool,
}

/// Digital pre-gain shared by capture validation, batch audio, live Nemotron,
/// and the meter (+12 dB = ~3.98x amplitude). This is not a hardware mic setting.
/// Automatic level normalization follows it, raising quiet speech or attenuating
/// loud speech before the limiter. Float multiplication here does not hard-clip;
/// clipping already present in the device samples cannot be repaired by gain.
pub const FIXED_INPUT_GAIN_DB: f32 = 12.0;

pub fn apply_gain_db(samples: &mut [f32], gain_db: f32) {
    if gain_db.abs() < 1e-4 || samples.is_empty() {
        return;
    }
    let factor = 10.0f32.powf(gain_db / 20.0);
    for s in samples.iter_mut() {
        *s *= factor;
    }
}

pub fn remove_dc_offset(samples: &mut [f32]) {
    if samples.is_empty() {
        return;
    }
    let mean = samples.iter().map(|&x| x as f64).sum::<f64>() / samples.len() as f64;
    for s in samples.iter_mut() {
        *s = (*s as f64 - mean) as f32;
    }
}

pub fn highpass_50hz(samples: &mut [f32], sample_rate: u32) {
    if samples.is_empty() || sample_rate < 100 {
        return;
    }
    let fc = 50.0f64;
    let fs = sample_rate as f64;
    if fc >= fs / 2.0 {
        return;
    }
    let w0 = 2.0 * std::f64::consts::PI * fc / fs;
    let cos_w0 = w0.cos();
    let sin_w0 = w0.sin();
    let alpha = sin_w0 * std::f64::consts::FRAC_1_SQRT_2;

    let b0 = (1.0 + cos_w0) / 2.0;
    let b1 = -(1.0 + cos_w0);
    let b2 = (1.0 + cos_w0) / 2.0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_w0;
    let a2 = 1.0 - alpha;

    let b0 = (b0 / a0) as f32;
    let b1 = (b1 / a0) as f32;
    let b2 = (b2 / a0) as f32;
    let a1 = (a1 / a0) as f32;
    let a2 = (a2 / a0) as f32;

    let mut s1 = 0.0f32;
    let mut s2 = 0.0f32;
    for s in samples.iter_mut() {
        let x = *s;
        let y = b0 * x + s1;
        s1 = b1 * x - a1 * y + s2;
        s2 = b2 * x - a2 * y;
        *s = y;
    }
}

/// Stop-time silence gate: was this capture effectively silent (nothing worth sending to
/// the engine)? `gain_factor` is the user's input gain applied in the judgement so a
/// raised gain rescues quiet speech.
///
/// Two conditions must BOTH hold to call it silent — the overall mean RMS over the whole
/// clip AND the loudest 20 ms frame must be below threshold. A long clip that is mostly
/// silence with one softly-spoken sentence has a mean dragged near zero, but its speech
/// frames stand well above the frame threshold; requiring both keeps that speech while
/// still rejecting true silence and steady low-level hiss. It errs toward keeping anything
/// with a speech-energy frame — a missed utterance is a worse failure than an occasional
/// noise burst. This mirrors FluidVoice's opt-in gate, which only skips a recording when
/// `peak && rms && maxFrameRMS` are all under threshold.
pub fn is_silent_capture(samples: &[f32], sample_rate: u32, gain_factor: f32) -> bool {
    if samples.is_empty() {
        return true;
    }
    let rms_of = |chunk: &[f32]| -> f32 {
        let energy: f32 = chunk
            .iter()
            .map(|&x| {
                let v = x * gain_factor;
                v * v
            })
            .sum();
        (energy / chunk.len() as f32).sqrt()
    };
    let total_rms = rms_of(samples);
    if total_rms >= 0.003 {
        return false;
    }
    // 20 ms frames: enough to catch a spoken word's energy, short enough that surrounding
    // silence cannot average it away.
    let frame = (sample_rate as usize / 50).max(1);
    let max_frame_rms = samples
        .chunks(frame)
        .map(rms_of)
        .fold(0.0f32, f32::max);
    max_frame_rms < 0.006
}

pub fn measure_p95_frame_rms(samples: &[f32], sample_rate: u32) -> f32 {
    if samples.is_empty() || sample_rate == 0 {
        return 0.0;
    }
    let frame_size = ((sample_rate as f32) * 0.020).round() as usize;
    if frame_size == 0 {
        return 0.0;
    }
    if samples.len() < frame_size {
        let sum_sq: f64 = samples.iter().map(|&x| (x as f64) * (x as f64)).sum();
        return (sum_sq / samples.len() as f64).sqrt() as f32;
    }

    let mut rms_values = Vec::with_capacity(samples.len() / frame_size);
    for chunk in samples.chunks(frame_size) {
        if chunk.len() >= frame_size / 2 {
            let sum_sq: f64 = chunk.iter().map(|&x| (x as f64) * (x as f64)).sum();
            let rms = (sum_sq / chunk.len() as f64).sqrt() as f32;
            rms_values.push(rms);
        }
    }
    if rms_values.is_empty() {
        return 0.0;
    }
    rms_values.sort_by(|a, b| a.total_cmp(b));
    let idx = ((rms_values.len() - 1) as f32 * 0.95).round() as usize;
    rms_values[idx.min(rms_values.len() - 1)]
}

/// RMS of the final `ms` milliseconds of `samples`. Diagnostic for end-of-dictation
/// truncation: a tail near the clip's speech level means capture ended mid-speech; a tail
/// near the noise floor means the last word was captured in full.
pub fn tail_rms(samples: &[f32], sample_rate: u32, ms: u32) -> f32 {
    let n = ((sample_rate as u64 * ms as u64) / 1000) as usize;
    let tail = &samples[samples.len().saturating_sub(n)..];
    if tail.is_empty() {
        return 0.0;
    }
    (tail.iter().map(|&x| x * x).sum::<f32>() / tail.len() as f32).sqrt()
}

pub fn normalize_target_rms(samples: &mut [f32], sample_rate: u32, target_rms: f32, max_gain: f32) -> f32 {
    let p95 = measure_p95_frame_rms(samples, sample_rate);
    if p95 < 1e-5 {
        return p95;
    }
    let gain = (target_rms / p95).min(max_gain);
    for s in samples.iter_mut() {
        *s *= gain;
    }
    p95
}

pub fn apply_tanh_limiter(samples: &mut [f32], threshold: f32, ceiling: f32) {
    if threshold <= 0.0 || ceiling <= threshold {
        return;
    }
    let range = ceiling - threshold;
    for s in samples.iter_mut() {
        let abs_val = s.abs();
        if abs_val > threshold {
            let sign = if *s >= 0.0 { 1.0 } else { -1.0 };
            let saturated = threshold + range * ((abs_val - threshold) / range).tanh();
            *s = sign * saturated;
        }
    }
}

pub fn process_speech_audio_chain(
    samples: &mut [f32],
    sample_rate: u32,
    input_gain_db: f32,
) -> PreprocessDiagnostics {
    if samples.is_empty() {
        return PreprocessDiagnostics {
            clipping_detected: false,
            clip_count: 0,
            p95_rms: 0.0,
            post_rms: 0.0,
            is_quiet: false,
        };
    }

    // 1. Clipping detection on the RAW capture, before any gain. Clipping happens at the
    // device, not after our own boost: measured post-gain, any peak above ~0.25 would read
    // as "clipping" under the fixed +12 dB and fire a false "speak further away" toast.
    let clip_count = samples.iter().filter(|&&s| s.abs() >= 0.999).count();
    let clipping_detected = clip_count > 0;

    // 2. Native input gain
    apply_gain_db(samples, input_gain_db);

    // 3. DC offset removal
    remove_dc_offset(samples);

    // 4. 50 Hz high-pass filter
    highpass_50hz(samples, sample_rate);

    // 5. p95 frame RMS target normalization (target 0.08, max gain 20x)
    const TARGET_RMS: f32 = 0.08;
    const MAX_GAIN: f32 = 20.0;
    let p95_rms = normalize_target_rms(samples, sample_rate, TARGET_RMS, MAX_GAIN);

    // 6. Soft-knee tanh limiter capping at 0.95
    apply_tanh_limiter(samples, 0.75, 0.95);

    // 7. Post-normalization RMS and quiet check
    let post_rms = measure_p95_frame_rms(samples, sample_rate);
    let is_quiet = p95_rms < 0.010;

    PreprocessDiagnostics {
        clipping_detected,
        clip_count,
        p95_rms,
        post_rms,
        is_quiet,
    }
}


// ---------------------------------------------------------------------------------------
// Streaming (chunk-by-chunk) preprocessing for the Nemotron live path.
//
// The batch chain above (`process_speech_audio_chain`) needs the whole clip: it measures the
// p95 frame RMS over everything before choosing a gain. The Nemotron live session feeds the
// recognizer ~150 ms at a time while the user is still speaking, so it cannot wait. Since
// v0.2.0 that path received the RAW microphone signal (no DC removal, no high-pass, no level
// normalization, no limiter) while every other engine received the processed signal.
// `StreamingPreprocessor` gives the live path the same stages, computed causally:
//   fixed pre-gain -> 50 Hz high-pass (stateful) -> speech-level tracking -> smoothly ramped
//   gain toward the same 0.08 target RMS -> the same soft-knee tanh limiter (0.75 / 0.95).
// The gain is interpolated per SAMPLE across each chunk (never stepped at a chunk boundary),
// which is what the earlier per-chunk-gain attempt got wrong ("streaming distortion").
// ---------------------------------------------------------------------------------------
const STREAM_TARGET_RMS: f32 = 0.08; // same as TARGET_RMS in process_speech_audio_chain
const STREAM_MAX_GAIN: f32 = 20.0; // same as MAX_GAIN in process_speech_audio_chain
const STREAM_MIN_GAIN: f32 = 0.05;
/// Minimum loudest-frame RMS for live level tracking, after pre-gain/filtering.
/// The old 0.01 floor missed quiet speech already accepted by the capture gate
/// (mean >= 0.003): e.g. a 0.002-amplitude voice at +12 dB has RMS ~0.0056.
/// Keep a floor above low-level noise so it cannot drive automatic gain upward.
const STREAM_GATE_RMS: f32 = 0.005;

/// Direct-form-II-transposed biquad coefficients [b0, b1, b2, a1, a2] of the same 2nd-order
/// 50 Hz Butterworth high-pass used by `highpass_50hz`.
fn highpass_50hz_coeffs(sample_rate: u32) -> Option<[f32; 5]> {
    if sample_rate < 100 {
        return None;
    }
    let fc = 50.0f64;
    let fs = sample_rate as f64;
    if fc >= fs / 2.0 {
        return None;
    }
    let w0 = 2.0 * std::f64::consts::PI * fc / fs;
    let cos_w0 = w0.cos();
    let sin_w0 = w0.sin();
    let alpha = sin_w0 * std::f64::consts::FRAC_1_SQRT_2;
    let b0 = (1.0 + cos_w0) / 2.0;
    let b1 = -(1.0 + cos_w0);
    let b2 = (1.0 + cos_w0) / 2.0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_w0;
    let a2 = 1.0 - alpha;
    Some([
        (b0 / a0) as f32,
        (b1 / a0) as f32,
        (b2 / a0) as f32,
        (a1 / a0) as f32,
        (a2 / a0) as f32,
    ])
}

/// RMS of the loudest 20 ms frame in `chunk` (frames shorter than half a frame are ignored,
/// matching `measure_p95_frame_rms`). Returns 0.0 when the chunk is too short to hold one.
fn loudest_frame_rms(chunk: &[f32], frame: usize) -> f32 {
    if frame == 0 {
        return 0.0;
    }
    let mut best = 0.0f32;
    for f in chunk.chunks(frame) {
        if f.len() >= (frame / 2).max(1) {
            let sum_sq: f64 = f.iter().map(|&x| (x as f64) * (x as f64)).sum();
            let rms = (sum_sq / f.len() as f64).sqrt() as f32;
            if rms > best {
                best = rms;
            }
        }
    }
    best
}

pub struct StreamingPreprocessor {
    pre_gain: f32,
    hp: Option<[f32; 5]>,
    z1: f32,
    z2: f32,
    primed: bool,
    /// Tracked speech level (RMS of the loudest recent frames); 0.0 until speech is seen.
    level: f32,
    /// Gain applied at the end of the previous chunk.
    gain: f32,
    frame: usize,
}

impl StreamingPreprocessor {
    /// Continuous filter for chunk-boundary planning without changing decoder
    /// gain. Keeping DC out of the energy scan prevents missed real pauses.
    pub(crate) fn highpass_only(&mut self, samples: &mut [f32]) { self.highpass(samples); }

    /// `sample_rate` is the rate of the chunks that will be passed to `process` (16000 for
    /// the Nemotron path). `pre_gain_db` is the fixed native input gain (FIXED_INPUT_GAIN_DB).
    pub fn new(sample_rate: u32, pre_gain_db: f32) -> Self {
        Self {
            pre_gain: 10f32.powf(pre_gain_db / 20.0),
            hp: highpass_50hz_coeffs(sample_rate),
            z1: 0.0,
            z2: 0.0,
            primed: false,
            level: 0.0,
            gain: 1.0,
            frame: ((sample_rate as f32) * 0.020).round() as usize,
        }
    }

    /// Stateful 50 Hz high-pass. The filter state is pre-loaded on the first sample so a DC
    /// offset (or any non-zero first sample) does not produce a start-up transient.
    fn highpass(&mut self, chunk: &mut [f32]) {
        let Some([b0, b1, b2, a1, a2]) = self.hp else {
            return;
        };
        if chunk.is_empty() {
            return;
        }
        if !self.primed {
            let x0 = chunk[0];
            self.z1 = -b0 * x0;
            self.z2 = b2 * x0;
            self.primed = true;
        }
        for s in chunk.iter_mut() {
            let x = *s;
            let y = b0 * x + self.z1;
            self.z1 = b1 * x - a1 * y + self.z2;
            self.z2 = b2 * x - a2 * y;
            *s = y;
        }
    }

    /// Process one chunk in place. Chunks may be any length; state carries across calls.
    pub fn process(&mut self, chunk: &mut [f32]) {
        if chunk.is_empty() {
            return;
        }
        if self.pre_gain != 1.0 {
            for s in chunk.iter_mut() {
                *s *= self.pre_gain;
            }
        }
        self.highpass(chunk);

        let loudest = loudest_frame_rms(chunk, self.frame);
        let first_speech = self.level == 0.0;
        if loudest >= STREAM_GATE_RMS {
            self.level = if first_speech {
                loudest
            } else if loudest > self.level {
                self.level + 0.5 * (loudest - self.level) // fast attack
            } else {
                self.level + 0.1 * (loudest - self.level) // slow release
            };
        }
        let target_gain = if self.level > 0.0 {
            (STREAM_TARGET_RMS / self.level).clamp(STREAM_MIN_GAIN, STREAM_MAX_GAIN)
        } else {
            1.0
        };
        // The very first speech chunk adopts its target gain immediately: the preceding audio
        // is noise-floor, so the step is inaudible. Afterwards the gain ramps per sample.
        let start_gain = if first_speech && self.level > 0.0 { target_gain } else { self.gain };
        let n = chunk.len() as f32;
        for (i, s) in chunk.iter_mut().enumerate() {
            let g = start_gain + (target_gain - start_gain) * ((i + 1) as f32 / n);
            *s *= g;
        }
        self.gain = target_gain;

        apply_tanh_limiter(chunk, 0.75, 0.95);
    }
}

#[allow(dead_code)]
const NORM_TARGET_PEAK: f32 = 0.95;
#[allow(dead_code)]
const NORM_MAX_GAIN: f32 = 15.0;
#[allow(dead_code)]
const NORM_EPS: f32 = 1e-4;

/// Peak-normalize `samples` toward `target_peak`, but never amplify by more
/// than `max_gain` (so a near-silent clip's noise floor is not blown up).
/// Near-silent input is left unchanged.
#[allow(dead_code)]
fn normalize_peak(samples: &mut [f32], target_peak: f32, max_gain: f32) {

    let peak = samples.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    if peak < NORM_EPS {
        return;
    }
    let gain = (target_peak / peak).min(max_gain);
    for s in samples.iter_mut() {
        *s = (*s * gain).clamp(-1.0, 1.0);
    }
}

/// Number of taps in the anti-aliasing low-pass. Odd so the filter has an integer delay.
/// Kept modest so the whole resample stays cheap even in an unoptimized debug build — the
/// kernel is precomputed once, so the hot loop is pure multiply-add with no transcendentals
/// (an earlier windowed-sinc-per-sample version made a debug build take ~30s per clip).
const RESAMPLE_LOWPASS_TAPS: usize = 31;

pub fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }

    // Anti-aliasing only matters when downsampling: decimating 48 kHz -> 16 kHz without a
    // low-pass first folds all 8-24 kHz energy back into the 0-8 kHz speech band as noise the
    // model then has to hear through. Upsampling can't fold anything in, so it is left alone.
    let kernel = if from_rate > to_rate {
        // Cutoff just under the OUTPUT Nyquist, expressed in cycles per INPUT sample.
        let fc = 0.5 * (to_rate as f64 / from_rate as f64) * 0.90;
        Some(lowpass_kernel(fc, RESAMPLE_LOWPASS_TAPS))
    } else {
        None
    };
    // Evaluate the same FIR only at positions used by interpolation. At 48 kHz
    // -> 16 kHz, two thirds of the old filtered samples were thrown away.
    // Preserve accumulation order and f32 rounding so the waveform is identical.
    let filtered_at = |index: usize| -> f32 {
        if let Some(kernel) = &kernel {
            let half = (kernel.len() / 2) as isize;
            let mut acc = 0.0f64;
            for (j, &weight) in kernel.iter().enumerate() {
                let source = index as isize + j as isize - half;
                if source >= 0 && (source as usize) < samples.len() {
                    acc += samples[source as usize] as f64 * weight;
                }
            }
            acc as f32
        } else {
            samples[index]
        }
    };

    // Unchanged from before: linear interpolation to the target rate. The output length formula
    // is identical, so the WAV (and everything downstream) is byte-for-byte the same shape.
    let ratio = from_rate as f64 / to_rate as f64;
    let output_len = (samples.len() as f64 / ratio) as usize;
    let mut output = Vec::with_capacity(output_len);

    for i in 0..output_len {
        let src_idx = i as f64 * ratio;
        let idx = src_idx as usize;
        let frac = src_idx - idx as f64;

        let sample = if idx + 1 < samples.len() {
            let a = filtered_at(idx);
            let b = if frac == 0.0 { a } else { filtered_at(idx + 1) };
            a as f64 * (1.0 - frac) + b as f64 * frac
        } else {
            filtered_at(idx.min(samples.len() - 1)) as f64
        };

        output.push(sample as f32);
    }

    output
}

/// Symmetric windowed-sinc low-pass FIR (Hamming window, normalized to unity DC gain).
/// `fc` is the cutoff in cycles per sample (0..0.5). The kernel is built once (a handful of
/// transcendentals), then the convolution is pure multiply-add — so this is fast even in a
/// debug build. Edges are handled by treating out-of-range input as zero.
#[cfg(test)]
fn lowpass(samples: &[f32], fc: f64, taps: usize) -> Vec<f32> {
    let kernel = lowpass_kernel(fc, taps);
    let half = (kernel.len() / 2) as isize;
    let n = samples.len() as isize;
    let mut out = vec![0.0f32; samples.len()];
    for (i, output) in out.iter_mut().enumerate() {
        let mut acc = 0.0f64;
        for (j, &kv) in kernel.iter().enumerate() {
            let src = i as isize + j as isize - half;
            if src >= 0 && src < n {
                acc += samples[src as usize] as f64 * kv;
            }
        }
        *output = acc as f32;
    }
    out
}

fn lowpass_kernel(fc: f64, taps: usize) -> Vec<f64> {
    let taps = taps | 1; // force odd for a symmetric, integer-delay filter
    let m = (taps - 1) as f64;
    let mut kernel = vec![0.0f64; taps];
    let mut sum = 0.0;
    for (n, k) in kernel.iter_mut().enumerate() {
        let d = n as f64 - m / 2.0;
        let sinc = if d.abs() < 1e-9 {
            2.0 * fc
        } else {
            (2.0 * std::f64::consts::PI * fc * d).sin() / (std::f64::consts::PI * d)
        };
        let window = 0.54 - 0.46 * (2.0 * std::f64::consts::PI * n as f64 / m).cos(); // Hamming
        let v = sinc * window;
        *k = v;
        sum += v;
    }
    for k in kernel.iter_mut() {
        *k /= sum; // unity gain at DC — the filter must not change loudness
    }

    kernel
}

/// Continuous equivalent of `resample`: retain the FIR history, lookahead, and
/// fractional sample position across microphone reads. Restarting the batch
/// filter every 150 ms zero-pads artificial edges and loses fractional samples.
pub struct StreamingResampler {
    ratio: f64,
    kernel: Vec<f64>,
    samples: Vec<f32>,
    base: usize,
    total: usize,
    next_output: usize,
}

impl StreamingResampler {
    pub fn new(from_rate: u32, to_rate: u32) -> Self {
        assert!(from_rate > 0 && to_rate > 0);
        let kernel = if from_rate > to_rate {
            lowpass_kernel(0.45 * to_rate as f64 / from_rate as f64, RESAMPLE_LOWPASS_TAPS)
        } else {
            vec![1.0]
        };
        Self { ratio: from_rate as f64 / to_rate as f64, kernel,
            samples: Vec::new(), base: 0, total: 0, next_output: 0 }
    }

    pub fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        self.samples.extend_from_slice(samples);
        self.total += samples.len();
        self.drain(false)
    }

    /// Flush only the true end of the recording with zero boundary conditions.
    pub fn finish(&mut self) -> Vec<f32> {
        self.drain(true)
    }

    fn filtered(&self, index: usize) -> f32 {
        let half = self.kernel.len() as isize / 2;
        self.kernel.iter().enumerate().map(|(j, &weight)| {
            let source = index as isize + j as isize - half;
            if source < 0 || source as usize >= self.total { 0.0 }
            else { self.samples[source as usize - self.base] as f64 * weight }
        }).sum::<f64>() as f32
    }

    fn drain(&mut self, finished: bool) -> Vec<f32> {
        let output_len = (self.total as f64 / self.ratio) as usize;
        let half = self.kernel.len() / 2;
        let mut output = Vec::new();
        while self.next_output < output_len {
            let position = self.next_output as f64 * self.ratio;
            let index = position as usize;
            if !finished && index + half + 1 >= self.total { break; }
            let a = self.filtered(index.min(self.total - 1));
            let fraction = position - index as f64;
            let b = if fraction == 0.0 { a }
                else { self.filtered((index + 1).min(self.total - 1)) };
            output.push((a as f64 * (1.0 - fraction) + b as f64 * fraction) as f32);
            self.next_output += 1;
        }
        let next_index = (self.next_output as f64 * self.ratio) as usize;
        let retain_from = next_index.saturating_sub(half).min(self.total);
        let discard = retain_from.saturating_sub(self.base);
        self.samples.drain(..discard);
        self.base += discard;
        output
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn long_capture_allocation_is_released_after_transcription() {
        let recorder = super::AudioRecorder::new();
        let second = recorder.source_sample_rate as usize * recorder.source_channels as usize;
        recorder.samples.lock().unwrap().resize(second * 80, 0.01);
        recorder.clear_samples();
        let samples = recorder.samples.lock().unwrap();
        assert!(samples.is_empty()); assert!(samples.capacity() <= second * 2);
    }
    use super::*;

    fn legacy_resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
        if from_rate == to_rate || samples.is_empty() { return samples.to_vec(); }
        let filtered = if from_rate > to_rate {
            lowpass(samples, 0.5 * (to_rate as f64 / from_rate as f64) * 0.90, RESAMPLE_LOWPASS_TAPS)
        } else { samples.to_vec() };
        let ratio = from_rate as f64 / to_rate as f64;
        (0..(samples.len() as f64 / ratio) as usize).map(|i| {
            let position = i as f64 * ratio;
            let index = position as usize;
            let fraction = position - index as f64;
            if index + 1 < filtered.len() {
                (filtered[index] as f64 * (1.0 - fraction)
                    + filtered[index + 1] as f64 * fraction) as f32
            } else { filtered[index.min(filtered.len() - 1)] }
        }).collect()
    }

    #[test]
    fn optimized_resampler_is_bit_identical_to_previous_filter() {
        for from_rate in [8_000, 16_000, 22_050, 44_100, 48_000, 96_000] {
            for to_rate in [16_000, 22_050, 48_000] {
                for len in [0, 1, 2, 17, 133, 8191] {
                    let samples: Vec<_> = (0..len).map(|i| (i as f32 * 0.037).sin() * 0.4).collect();
                    let expected = legacy_resample(&samples, from_rate, to_rate);
                    let actual = resample(&samples, from_rate, to_rate);
                    assert_eq!(actual.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                        expected.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                        "from={from_rate} to={to_rate} len={len}");
                }
            }
        }
    }

    #[test]
    #[ignore = "manual latency measurement; no CI timing threshold"]
    fn benchmark_resample_without_waveform_changes() {
        let samples: Vec<_> = (0..48_000 * 80).map(|i| (i as f32 * 0.037).sin() * 0.4).collect();
        let started = std::time::Instant::now();
        let expected = legacy_resample(&samples, 48_000, 16_000);
        let old_ms = started.elapsed().as_millis();
        let started = std::time::Instant::now();
        let actual = resample(&samples, 48_000, 16_000);
        let new_ms = started.elapsed().as_millis();
        assert_eq!(actual, expected);
        println!("80s audio @48kHz: legacy FIR={old_ms}ms, optimized FIR={new_ms}ms; identical samples");
    }

    #[test]
    fn streaming_resampling_matches_whole_audio_at_arbitrary_boundaries() {
        for from_rate in [8_000, 16_000, 44_100, 48_000] {
            let samples: Vec<_> = (0..10_031).map(|i| {
                let t = i as f32 / from_rate as f32;
                (t * 997.0 * 2.0 * std::f32::consts::PI).sin() * 0.3
                    + (t * 11_003.0 * 2.0 * std::f32::consts::PI).sin() * 0.1
            }).collect();
            let expected = resample(&samples, from_rate, 16_000);
            for chunk_size in [1, 17, 1023, 2399] {
                let mut resampler = StreamingResampler::new(from_rate, 16_000);
                let mut actual = Vec::new();
                for chunk in samples.chunks(chunk_size) {
                    actual.extend(resampler.process(chunk));
                    assert!(resampler.samples.len() <= RESAMPLE_LOWPASS_TAPS * 2 + 4);
                }
                actual.extend(resampler.finish());
                assert_eq!(actual.len(), expected.len(), "rate={from_rate}, chunk={chunk_size}");
                for (a, b) in actual.iter().zip(&expected) {
                    assert!((a - b).abs() < 1e-6, "rate={from_rate}, chunk={chunk_size}: {a} != {b}");
                }
                assert!(resampler.finish().is_empty());
            }
        }
    }

    #[test]
    fn test_smooth_band() {
        // Rising toward a higher target uses the fast attack coefficient.
        assert!((smooth_band(0.0, 1.0, 0.35, 0.08) - 0.35).abs() < 1e-6);
        // Falling toward a lower target uses the slow release coefficient.
        assert!((smooth_band(1.0, 0.0, 0.35, 0.08) - 0.92).abs() < 1e-6);
        // One attack step (rise) moves more than one release step (fall) of equal distance.
        let rose = smooth_band(0.0, 1.0, 0.35, 0.08) - 0.0;
        let fell = 1.0 - smooth_band(1.0, 0.0, 0.35, 0.08);
        assert!(rose > fell);
        // Clamped to [0, 1] even with overshooting inputs.
        assert_eq!(smooth_band(1.0, 5.0, 1.0, 0.08), 1.0);
        assert_eq!(smooth_band(0.0, -5.0, 1.0, 1.0), 0.0);
    }

    #[test]
    fn test_agc_update() {
        // A louder block is now approached gradually, not snapped to: 0.5 + (2.0-0.5)*0.3.
        assert!((agc_update(0.5, 2.0, 0.995, 0.3) - 0.95).abs() < 1e-6);
        // A single transient can't capture the reference — one step stays well short of it.
        assert!(agc_update(0.5, 2.0, 0.995, 0.3) < 2.0);
        // A sustained louder level keeps climbing toward the block max over repeated blocks.
        let mut m = 0.5;
        for _ in 0..20 { m = agc_update(m, 2.0, 0.995, 0.3); }
        assert!(m > 1.9);
        // A quieter block decays the running peak gently (1.0 * 0.995).
        assert!((agc_update(1.0, 0.1, 0.995, 0.3) - 0.995).abs() < 1e-6);
        // Never falls below the floor.
        assert!((agc_update(0.0, 0.0, 0.995, 0.3) - 0.0001).abs() < 1e-9);
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt()
    }

    fn tone(freq: f64, sr: u32, secs: f64) -> Vec<f32> {
        let n = (sr as f64 * secs) as usize;
        (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * freq * i as f64 / sr as f64).sin() as f32)
            .collect()
    }

    #[test]
    fn test_resample_passthrough_and_length() {
        // Equal rates: untouched.
        let s = vec![0.1f32, 0.2, 0.3, 0.4];
        assert_eq!(resample(&s, 16000, 16000), s);
        // The output-length contract must be exactly what it was before (downstream depends on it).
        assert_eq!(resample(&vec![0.0f32; 48000], 48000, 16000).len(), 16000);
        assert!(resample(&[], 48000, 16000).is_empty());
    }

    #[test]
    fn test_resample_anti_aliases_but_keeps_speech_band() {
        // The whole point of the fix. A 3 kHz tone is well inside the 8 kHz output Nyquist and
        // must survive; a 11 kHz tone is above it and, under naive decimation, would fold down
        // into the speech band as noise — the anti-aliased resampler must crush it instead.
        let low = resample(&tone(3000.0, 48000, 1.0), 48000, 16000);
        let high = resample(&tone(11000.0, 48000, 1.0), 48000, 16000);
        // Trim filter edges before measuring.
        let low_rms = rms(&low[200..low.len() - 200]);
        let high_rms = rms(&high[200..high.len() - 200]);
        assert!(low_rms > 0.5, "3kHz speech-band tone should pass, rms {}", low_rms);
        assert!(high_rms < 0.1, "11kHz tone should be attenuated, rms {}", high_rms);
    }

    #[test]
    fn test_resample_is_fast_enough_in_debug() {
        // Guard against ever reintroducing the slow resampler that made a debug build take ~30s
        // to paste. A 30s dictation at 48 kHz must resample in well under a second even here,
        // unoptimized. (Release is ~30x faster still.)
        let input: Vec<f32> = (0..48000 * 30).map(|i| (i as f32 * 0.01).sin() * 0.3).collect();
        let start = std::time::Instant::now();
        let out = resample(&input, 48000, 16000);
        let elapsed = start.elapsed();
        assert_eq!(out.len(), 16000 * 30);
        assert!(elapsed < std::time::Duration::from_secs(2), "resample too slow: {:?}", elapsed);
    }

    #[test]
    fn test_resolve_default_uses_default_device() {
        let avail = vec!["Built-in".to_string(), "USB Mic".to_string()];
        let r = resolve_mic("default", &avail, Some("Built-in")).unwrap();
        assert_eq!(r, MicResolution { target: "Built-in".to_string(), fell_back: false });
    }

    #[test]
    fn test_resolve_named_present() {
        let avail = vec!["Built-in".to_string(), "USB Mic".to_string()];
        let r = resolve_mic("USB Mic", &avail, Some("Built-in")).unwrap();
        assert_eq!(r, MicResolution { target: "USB Mic".to_string(), fell_back: false });
    }

    #[test]
    fn test_resolve_named_absent_falls_back_to_default() {
        let avail = vec!["Built-in".to_string()];
        let r = resolve_mic("USB Mic", &avail, Some("Built-in")).unwrap();
        assert_eq!(r, MicResolution { target: "Built-in".to_string(), fell_back: true });
    }

    #[test]
    fn test_resolve_named_absent_no_default_errors() {
        let avail: Vec<String> = vec![];
        assert!(resolve_mic("USB Mic", &avail, None).is_err());
    }

    #[test]
    fn test_resolve_default_no_default_device_errors() {
        let avail: Vec<String> = vec![];
        assert!(resolve_mic("default", &avail, None).is_err());
    }

    #[test]
    fn test_can_reuse_stream() {
        // healthy stream built for the same setting -> reuse
        assert!(can_reuse_stream(true, false, Some("default"), "default"));
        // no stream yet -> rebuild
        assert!(!can_reuse_stream(false, false, None, "default"));
        // stream errored (e.g. device unplugged) -> rebuild
        assert!(!can_reuse_stream(true, true, Some("default"), "default"));
        // different setting requested -> rebuild
        assert!(!can_reuse_stream(true, false, Some("default"), "USB Mic"));
    }

    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
    }

    #[test]
    fn test_normalize_boosts_quiet_signal() {
        let mut s = vec![0.1, -0.05, 0.08];
        normalize_peak(&mut s, 0.95, 15.0);
        // peak 0.1 -> gain 9.5 (< cap) -> new peak ~0.95
        assert!((peak(&s) - 0.95).abs() < 0.02, "peak was {}", peak(&s));
    }

    #[test]
    fn test_normalize_respects_max_gain_on_very_quiet() {
        let mut s = vec![0.02, -0.01];
        normalize_peak(&mut s, 0.95, 15.0);
        // peak 0.02 would need 47x; capped at 15 -> new peak ~0.30
        assert!(peak(&s) <= 0.95);
        assert!((peak(&s) - 0.30).abs() < 0.02, "peak was {}", peak(&s));
    }

    #[test]
    fn test_normalize_loud_signal_no_clip() {
        let mut s = vec![0.98, -0.9];
        normalize_peak(&mut s, 0.95, 15.0);
        assert!(peak(&s) <= 1.0);
        assert!((peak(&s) - 0.95).abs() < 0.02, "peak was {}", peak(&s));
    }

    #[test]
    fn test_normalize_silent_and_empty_unchanged() {
        let mut silent = vec![0.0, 0.0, 0.0];
        normalize_peak(&mut silent, 0.95, 15.0);
        assert_eq!(silent, vec![0.0, 0.0, 0.0]);

        let mut empty: Vec<f32> = vec![];
        normalize_peak(&mut empty, 0.95, 15.0); // must not panic
        assert!(empty.is_empty());
    }

    #[test]
    fn test_apply_gain_db() {
        let mut s = vec![0.1f32, -0.2, 0.5];
        apply_gain_db(&mut s, 0.0);
        assert_eq!(s, vec![0.1f32, -0.2, 0.5]);

        apply_gain_db(&mut s, 6.0205999);
        assert!((s[0] - 0.2).abs() < 1e-4);
        assert!((s[1] - (-0.4)).abs() < 1e-4);
        assert!((s[2] - 1.0).abs() < 1e-4);

        apply_gain_db(&mut s, -6.0205999);
        assert!((s[0] - 0.1).abs() < 1e-4);
        assert!((s[1] - (-0.2)).abs() < 1e-4);
        assert!((s[2] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn test_fixed_input_gain_lifts_quiet_speech_fourfold() {
        // The native fixed gain replaces the removed slider: +12 dB must equal ~4x
        // amplitude, lifting quiet speech clear of the silence gate with headroom
        // left in float samples for the later normalization/limiter stages.
        assert!((FIXED_INPUT_GAIN_DB - 12.0).abs() < 1e-6);
        let mut s = vec![0.05f32, -0.05, 0.1];
        apply_gain_db(&mut s, FIXED_INPUT_GAIN_DB);
        let expected = 10f32.powf(12.0 / 20.0); // ~3.98x
        assert!((s[0] - 0.05 * expected).abs() < 1e-6, "first sample was {}", s[0]);
        assert!((s[2] - 0.1 * expected).abs() < 1e-6, "third sample was {}", s[2]);
    }

    #[test]
    fn test_remove_dc_offset() {
        let mut s = vec![1.2f32, 1.4, 0.8, 1.0];
        remove_dc_offset(&mut s);
        let mean: f32 = s.iter().sum::<f32>() / s.len() as f32;
        assert!(mean.abs() < 1e-6);
        assert!((s[0] - 0.1).abs() < 1e-6);
        assert!((s[2] - (-0.3)).abs() < 1e-6);

        let mut empty: Vec<f32> = vec![];
        remove_dc_offset(&mut empty);
        assert!(empty.is_empty());
    }

    #[test]
    fn test_highpass_50hz() {
        let mut hi = tone(1000.0, 16000, 0.5);
        highpass_50hz(&mut hi, 16000);
        let hi_rms = rms(&hi[320..]);
        assert!(hi_rms > 0.65, "1000 Hz tone should pass, rms was {}", hi_rms);

        let mut lo = tone(10.0, 16000, 0.5);
        highpass_50hz(&mut lo, 16000);
        let lo_rms = rms(&lo[320..]);
        assert!(lo_rms < 0.15, "10 Hz sub-bass tone should be attenuated, rms was {}", lo_rms);

        let mut dc = vec![0.8f32; 3200];
        highpass_50hz(&mut dc, 16000);
        let end_rms = rms(&dc[2000..]);
        assert!(end_rms < 0.05, "DC should be filtered out, tail rms was {}", end_rms);
    }

    #[test]
    fn test_measure_p95_frame_rms() {
        let frame_len = 320;
        let mut samples = vec![0.0f32; 90 * frame_len];
        samples.extend(vec![0.2f32; 10 * frame_len]);

        let p95 = measure_p95_frame_rms(&samples, 16000);
        assert!((p95 - 0.2).abs() < 0.01, "p95 was {}", p95);
    }

    #[test]
    fn test_silence_gate_rejects_true_silence() {
        let silence = vec![0.0f32; 16000 * 2];
        assert!(is_silent_capture(&silence, 16000, 1.0));
        // Gain must not resurrect an all-zero clip.
        assert!(is_silent_capture(&silence, 16000, 4.0));
        // Steady hiss below both thresholds is silent too.
        let hiss = vec![0.0008f32; 16000 * 2];
        assert!(is_silent_capture(&hiss, 16000, 1.0));
    }

    #[test]
    fn test_silence_gate_keeps_quiet_speech_in_long_clip() {
        // Mostly silence with one softly-spoken sentence: the whole-clip mean RMS falls
        // under the mean threshold (the old single-condition gate rejected this as
        // "silent"), but the speech's 20 ms frames stand well above the frame threshold.
        let mut samples = vec![0.0f32; 16000 * 40];
        let tone: Vec<f32> = (0..6400)
            .map(|i| 0.02 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16000.0).sin())
            .collect();
        samples.extend(tone);

        let gain = 10f32.powf(0.0 / 20.0); // unity: no input gain helping
        assert!(!is_silent_capture(&samples, 16000, gain), "quiet speech must be captured");
    }

    #[test]
    fn test_silence_gate_gain_rescues_sustained_quiet_speech() {
        // Sustained very quiet speech: both mean and frame RMS sit under threshold at
        // unity gain, but +12 dB of user input gain lifts the mean over the gate — this is
        // the "input gain does nothing" bug (gate ran before gain).
        let quiet: Vec<f32> = (0..16000 * 2)
            .map(|i| 0.002 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 16000.0).sin())
            .collect();
        assert!(is_silent_capture(&quiet, 16000, 1.0), "baseline: too quiet at unity gain");
        assert!(
            !is_silent_capture(&quiet, 16000, 4.0),
            "input gain must lift quiet speech past the gate"
        );
    }

    #[test]
    fn test_normalize_target_rms() {
        let frame_len = 320;
        let mut samples = vec![0.02f32; 100 * frame_len];
        let pre_p95 = normalize_target_rms(&mut samples, 16000, 0.08, 20.0);
        assert!((pre_p95 - 0.02).abs() < 1e-4);
        let post_p95 = measure_p95_frame_rms(&samples, 16000);
        assert!((post_p95 - 0.08).abs() < 1e-3, "post_p95 was {}", post_p95);

        let mut very_quiet = vec![0.001f32; 100 * frame_len];
        normalize_target_rms(&mut very_quiet, 16000, 0.08, 20.0);
        let post_quiet_p95 = measure_p95_frame_rms(&very_quiet, 16000);
        assert!((post_quiet_p95 - 0.02).abs() < 1e-3);
    }

    #[test]
    fn test_apply_tanh_limiter() {
        let mut s = vec![0.5f32, -0.75, 0.85, 1.5, 10.0, -10.0];
        apply_tanh_limiter(&mut s, 0.75, 0.95);

        assert_eq!(s[0], 0.5);
        assert_eq!(s[1], -0.75);

        assert!(s[2] > 0.75 && s[2] < 0.85);
        assert!(s[3] > 0.90 && s[3] < 0.95);
        assert!(s[4] <= 0.95 && s[4] > 0.94);
        assert!(s[5] >= -0.95 && s[5] < -0.94);
    }

    #[test]
    fn test_process_speech_audio_chain() {
        let mut samples = tone(400.0, 16000, 2.0);
        samples[100] = 1.05;

        let diag = process_speech_audio_chain(&mut samples, 16000, 0.0);
        assert!(diag.clipping_detected);
        assert!(diag.clip_count >= 1);
        assert!(diag.post_rms > 0.05 && diag.post_rms <= 0.10);
        assert!(!diag.is_quiet);

        for &x in &samples {
            assert!(x.abs() <= 0.9501, "sample {} exceeded ceiling 0.95", x);
        }

        let mut quiet_samples = tone(400.0, 16000, 1.0);
        for s in quiet_samples.iter_mut() {
            *s *= 0.005;
        }
        let quiet_diag = process_speech_audio_chain(&mut quiet_samples, 16000, 0.0);
        assert!(quiet_diag.is_quiet);
        assert!(!quiet_diag.clipping_detected);
    }

    #[test]
    fn test_clipping_is_judged_on_raw_capture_not_after_gain() {
        // A loud-but-unclipped mic (peak 0.5) becomes 1.99 after +12 dB. That is our boost,
        // not mic clipping, and must not raise the clipping flag.
        let mut loud = tone(400.0, 16000, 1.0);
        for s in loud.iter_mut() {
            *s *= 0.5;
        }
        let d = process_speech_audio_chain(&mut loud, 16000, FIXED_INPUT_GAIN_DB);
        assert!(!d.clipping_detected, "false clipping from our own gain");
        assert_eq!(d.clip_count, 0);

        // A genuinely clipped capture is still caught with the gain applied.
        let mut clipped = tone(400.0, 16000, 1.0);
        clipped[100] = 1.0;
        let d = process_speech_audio_chain(&mut clipped, 16000, FIXED_INPUT_GAIN_DB);
        assert!(d.clipping_detected);
        assert!(d.clip_count >= 1);
    }

    #[test]
    fn test_tail_rms() {
        let mut s = vec![0.0f32; 16000];
        for x in s[8000..].iter_mut() {
            *x = 0.5;
        }
        assert!((tail_rms(&s, 16000, 150) - 0.5).abs() < 1e-6); // tail inside the loud part
        let mut quiet_end = s.clone();
        quiet_end.extend(std::iter::repeat(0.0f32).take(3200)); // 200 ms of silence appended
        assert_eq!(tail_rms(&quiet_end, 16000, 150), 0.0);
        assert_eq!(tail_rms(&[], 16000, 150), 0.0);
        assert!((tail_rms(&[0.3, 0.3], 16000, 150) - 0.3).abs() < 1e-6); // shorter than window
    }

    #[test]
    fn test_stop_and_save_keeps_samples_for_live_drain() {
        // Regression: stop_and_save used to clear the raw buffer, so the Nemotron session's
        // final `get_raw_samples_from(last_read)` drain (which runs afterwards) returned
        // nothing and the last poll interval of audio never reached the live stream.
        let mut rec = AudioRecorder::new();
        rec.set_source_format_for_test(16000, 1);
        let pushed = tone(300.0, 16000, 1.0);
        rec.push_raw_samples_for_test(&pushed);
        let out = std::env::temp_dir().join("typr_test_live_drain.wav");
        let res = rec.stop_and_save(&out, None);
        let _ = std::fs::remove_file(&out);
        assert!(res.is_ok(), "stop_and_save failed: {:?}", res.err());

        let (tail, end, rate, ch) = rec.get_raw_samples_from(0);
        assert_eq!(tail.len(), pushed.len(), "raw buffer must survive stop_and_save");
        assert_eq!(end, pushed.len());
        assert_eq!((rate, ch), (16000, 1));
        // The unread tail after a partial read is still retrievable.
        let (rest, _, _, _) = rec.get_raw_samples_from(12000);
        assert_eq!(rest.len(), pushed.len() - 12000);

        rec.clear_samples();
        assert!(rec.get_raw_samples_from(0).0.is_empty());
    }

    #[test]
    fn stopped_capture_validates_keeps_tail_and_can_save_later() {
        let mut rec = AudioRecorder::new();
        rec.set_source_format_for_test(48000, 2);
        assert_eq!(rec.stop_capture().unwrap_err(), "No audio captured");
        rec.push_raw_samples_for_test(&vec![0.0; 48000 * 2]);
        assert_eq!(rec.stop_capture().unwrap_err(), "Audio was silent");
        rec.clear_samples();
        let mono = tone(300.0, 48000, 1.0);
        let stereo: Vec<_> = mono.iter().flat_map(|&x| [x, x]).collect();
        rec.push_raw_samples_for_test(&stereo);
        assert_eq!(rec.stop_capture().unwrap(), 1.0);
        assert_eq!(rec.get_raw_samples_from(90000).0, stereo[90000..]);
        let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("typr-deferred-capture-{id}.wav"));
        rec.save_capture(&path, None).unwrap();
        let mut reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().sample_rate, 16000);
        let actual: Vec<_> = reader.samples::<i16>().map(Result::unwrap).collect();
        let mut expected = resample(&mono, 48000, 16000);
        process_speech_audio_chain(&mut expected, 16000, FIXED_INPUT_GAIN_DB);
        expected.extend(vec![0.0; 6400]);
        assert_eq!(actual, expected.iter().map(|x| (x.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).collect::<Vec<_>>());
        drop(reader);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(rec.get_raw_samples_from(0).0, stereo);
    }

    #[test]
    fn review_audio_round_trips_raw_device_samples_and_refuses_overwrite() {
        let mut rec = AudioRecorder::new();
        rec.set_source_format_for_test(44100, 2);
        let samples: Vec<_> = (0..4410).map(|i| (i as f32 * 0.07).sin() * 0.1).collect();
        rec.push_raw_samples_for_test(&samples);
        let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("typr-review-capture-{id}.wav"));
        let capture = rec.review_capture();
        rec.clear_samples();
        capture.save(&path).unwrap();
        assert!(rec.save_review_capture(&path).is_err());
        let mut reader = hound::WavReader::open(&path).unwrap();
        assert_eq!((reader.spec().sample_rate, reader.spec().channels), (44100, 2));
        assert_eq!(reader.samples::<f32>().map(Result::unwrap).collect::<Vec<_>>(), samples);
        drop(reader);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn review_audio_long_capture_uses_buffered_writes_without_changing_samples() {
        let mut rec = AudioRecorder::new();
        rec.set_source_format_for_test(48000, 2);
        let samples: Vec<f32> = (0..(72 * 48000 * 2))
            .map(|i| ((i % 1000) as f32 - 500.0) / 1000.0).collect();
        rec.push_raw_samples_for_test(&samples);
        let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("typr-review-long-{id}.wav"));
        let started = std::time::Instant::now();
        rec.save_review_capture(&path).unwrap();
        println!("REVIEW SAVE BENCH {} bytes in {}ms", std::fs::metadata(&path).unwrap().len(), started.elapsed().as_millis());
        let mut reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().sample_format, hound::SampleFormat::Float);
        assert_eq!(reader.samples::<f32>().map(Result::unwrap).collect::<Vec<_>>(), samples);
        drop(reader);
        std::fs::remove_file(&path).unwrap();
    }

    fn stream_rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt()
    }

    fn stream_run(p: &mut StreamingPreprocessor, input: &[f32], chunk: usize) -> Vec<f32> {
        let mut out = input.to_vec();
        for c in out.chunks_mut(chunk) {
            p.process(c);
        }
        out
    }

    #[test]
    fn test_streaming_highpass_matches_batch_highpass() {
        // Starting from a zero first sample the stateful filter must equal the batch filter
        // exactly, no matter how the signal is chunked.
        let mut sig = tone(180.0, 16000, 1.0);
        sig[0] = 0.0;
        let mut batch = sig.clone();
        highpass_50hz(&mut batch, 16000);
        for chunk in [1usize, 37, 320, 2400, 16000] {
            let mut p = StreamingPreprocessor::new(16000, 0.0);
            let mut live = sig.clone();
            for c in live.chunks_mut(chunk) {
                p.highpass(c);
            }
            let maxdiff = batch.iter().zip(&live).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(maxdiff < 1e-5, "chunk {chunk}: max diff {maxdiff}");
        }
    }

    #[test]
    fn test_streaming_removes_dc_offset_from_the_first_sample() {
        let mut p = StreamingPreprocessor::new(16000, 0.0);
        let mut dc = vec![0.1f32; 2400];
        p.highpass(&mut dc);
        let worst = dc.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "DC leaked through: {worst}");
    }

    #[test]
    fn test_streaming_converges_to_target_level() {
        // A quiet tone (RMS ~0.0212) must be brought to the batch target RMS of 0.08.
        let mut sig = tone(500.0, 16000, 3.0);
        for s in sig.iter_mut() {
            *s *= 0.03;
        }
        let mut p = StreamingPreprocessor::new(16000, 0.0);
        let out = stream_run(&mut p, &sig, 2400);
        let last = &out[out.len() - 16000..];
        let r = stream_rms(last);
        assert!((0.075..=0.085).contains(&r), "steady-state RMS {r}");
    }

    #[test]
    fn test_streaming_attenuates_loud_input_and_never_exceeds_ceiling() {
        let mut sig = tone(500.0, 16000, 2.0);
        for s in sig.iter_mut() {
            *s *= 0.9;
        }
        let mut p = StreamingPreprocessor::new(16000, FIXED_INPUT_GAIN_DB);
        let out = stream_run(&mut p, &sig, 2400);
        let peak = out.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
        assert!(peak <= 0.95 + 1e-6, "peak {peak} exceeds limiter ceiling");
        assert!(out.iter().all(|x| x.is_finite()));
        let r = stream_rms(&out[out.len() - 8000..]);
        assert!(r < 0.2, "loud input not brought down toward target: {r}");
    }

    #[test]
    fn fixed_gain_quiet_speech_receives_level_normalization_in_batch_and_live() {
        let quiet: Vec<_> = tone(220.0, 16000, 2.0).iter().map(|x| x * 0.002).collect();
        let gain = 10f32.powf(FIXED_INPUT_GAIN_DB / 20.0);
        assert!(!is_silent_capture(&quiet, 16000, gain));
        let mut batch = quiet.clone();
        let diagnostics = process_speech_audio_chain(&mut batch, 16000, FIXED_INPUT_GAIN_DB);
        assert!(!diagnostics.clipping_detected);
        assert!((0.075..=0.085).contains(&diagnostics.post_rms));
        for chunk_size in [320, 2400, 6400] {
            let mut pre = StreamingPreprocessor::new(16000, FIXED_INPUT_GAIN_DB);
            let live = stream_run(&mut pre, &quiet, chunk_size);
            assert!(pre.level > 0.0, "quiet speech ignored at chunk size {chunk_size}");
            let final_rms = stream_rms(&live[live.len() - 16000..]);
            assert!((0.075..=0.085).contains(&final_rms),
                "quiet speech under-amplified: chunk={chunk_size}, rms={final_rms}");
            assert!(live.iter().all(|x| x.is_finite() && x.abs() <= 0.95));
        }
    }

    #[test]
    fn test_streaming_is_transparent_at_chunk_boundaries() {
        // After convergence the gain is constant, so a chunk boundary must not introduce any
        // jump larger than the signal's own sample-to-sample movement.
        let mut sig = tone(300.0, 16000, 4.0);
        for s in sig.iter_mut() {
            *s *= 0.05;
        }
        let chunk = 2400;
        let mut p = StreamingPreprocessor::new(16000, 0.0);
        let out = stream_run(&mut p, &sig, chunk);
        let mut interior = 0.0f32;
        let mut boundary = 0.0f32;
        for i in (chunk * 8)..(out.len() - 1) {
            let d = (out[i + 1] - out[i]).abs();
            if (i + 1) % chunk == 0 {
                boundary = boundary.max(d);
            } else {
                interior = interior.max(d);
            }
        }
        assert!(boundary <= interior * 1.05, "boundary jump {boundary} vs interior {interior}");
    }

    #[test]
    fn test_streaming_output_is_independent_of_chunk_size() {
        let mut sig = tone(400.0, 16000, 3.0);
        for s in sig.iter_mut() {
            *s *= 0.02;
        }
        let mut levels = vec![];
        for chunk in [160usize, 2400, 6400] {
            let mut p = StreamingPreprocessor::new(16000, 0.0);
            let out = stream_run(&mut p, &sig, chunk);
            levels.push(stream_rms(&out[out.len() - 16000..]));
        }
        let (lo, hi) = levels.iter().fold((f32::MAX, 0.0f32), |(l, h), &v| (l.min(v), h.max(v)));
        assert!(hi / lo < 1.1, "chunk-size dependence: {levels:?}");
    }

    #[test]
    fn test_streaming_silence_never_raises_gain_or_initialises_level() {
        // Noise-only input (below the gate after pre-gain) must stay at unity gain...
        let mut p = StreamingPreprocessor::new(16000, FIXED_INPUT_GAIN_DB);
        let mut seed = 99u32;
        let noise: Vec<f32> = (0..32000)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (((seed >> 8) as f32 / 16777216.0) * 2.0 - 1.0) * 0.0015
            })
            .collect();
        let _ = stream_run(&mut p, &noise, 2400);
        assert_eq!(p.level, 0.0);
        assert_eq!(p.gain, 1.0);

        // ...and silence AFTER speech must hold the speech-derived gain, not pump it upward.
        let mut p = StreamingPreprocessor::new(16000, 0.0);
        let mut speech = tone(500.0, 16000, 1.0);
        for s in speech.iter_mut() {
            *s *= 0.05;
        }
        let _ = stream_run(&mut p, &speech, 2400);
        let gain_after_speech = p.gain;
        let _ = stream_run(&mut p, &vec![0.0f32; 32000], 2400);
        assert!((p.gain - gain_after_speech).abs() < 1e-6);
    }

    #[test]
    fn test_streaming_handles_empty_and_tiny_chunks() {
        let mut p = StreamingPreprocessor::new(16000, FIXED_INPUT_GAIN_DB);
        p.process(&mut []);
        let mut tiny = [0.2f32, -0.1, 0.05];
        p.process(&mut tiny);
        assert!(tiny.iter().all(|x| x.is_finite()));
        let mut none = StreamingPreprocessor::new(50, 0.0); // rate too low for the filter
        let mut c = [0.3f32; 10];
        none.process(&mut c);
        assert!(c.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn test_tail_silence_padding() {
        let mut samples = vec![0.5f32; 16000];
        let tail_silence_samples = (16000.0 * 0.400) as usize;
        samples.extend(std::iter::repeat(0.0f32).take(tail_silence_samples));
        assert_eq!(samples.len(), 16000 + 6400);
        for &s in &samples[16000..] {
            assert_eq!(s, 0.0);
        }
    }
}

