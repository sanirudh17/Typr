use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{WavSpec, WavWriter};
use rustfft::{Fft, FftPlanner, num_complex::Complex};
use std::sync::Arc as StdArc;
use std::sync::{Arc, Mutex};
use std::path::PathBuf;

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
}

/// Asymmetric temporal EMA: fast attack (rising toward a higher target), slow release
/// (falling toward a lower target). Result clamped to [0.0, 1.0].
pub fn smooth_band(prev: f32, target: f32, attack: f32, release: f32) -> f32 {
    let coeff = if target > prev { attack } else { release };
    (prev + (target - prev) * coeff).clamp(0.0, 1.0)
}

/// Running peak for AGC: rises instantly to a new higher block peak, otherwise decays
/// gently toward the (lower) current block max. Never falls below a small floor so we
/// never divide by ~zero.
pub fn agc_update(prev_max: f32, block_max: f32, decay: f32) -> f32 {
    const FLOOR: f32 = 0.0001;
    (prev_max * decay).max(block_max).max(FLOOR)
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
        }
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
        let stream_errored = self.stream_errored.clone();

        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut buf = samples.lock().unwrap();
                    buf.extend_from_slice(data);

                    let rms = (data.iter().map(|&x| x * x).sum::<f32>() / data.len() as f32).sqrt();
                    let normalized_amp = (rms * 8.0).min(1.0);

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
                        
                        // Spatial smoothing to make the bars move together seamlessly
                        let mut smoothed_amplitudes = vec![0.0; num_bands];
                        let mut max_amplitude = 0.0001f32;
                        
                        for i in 0..num_bands {
                            let mut val = amplitudes[i] * 0.4;
                            if i > 0 { val += amplitudes[i - 1] * 0.2; }
                            if i > 1 { val += amplitudes[i - 2] * 0.1; }
                            if i + 1 < num_bands { val += amplitudes[i + 1] * 0.2; }
                            if i + 2 < num_bands { val += amplitudes[i + 2] * 0.1; }
                            
                            smoothed_amplitudes[i] = val;
                            if val > max_amplitude {
                                max_amplitude = val;
                            }
                        }
                        
                        let noise_gate = 0.003; 
                        let is_speaking = block_rms > noise_gate;
                        
                        for band in 0..num_bands {
                            if !is_speaking {
                                bands[band] = 0.0;
                            } else {
                                let normalized = smoothed_amplitudes[band] / max_amplitude;
                                let shape = normalized.powi(2); // smooth rounded peak instead of isolated harsh spikes
                                let volume_factor = ((block_rms - noise_gate) * 50.0).min(1.0);
                                let pitch_height_boost = 1.0 + (band as f32 * 0.05); // taller bars for higher pitch
                                bands[band] = (shape * volume_factor * pitch_height_boost).min(1.0);
                            }
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

    pub fn stop_and_save(&mut self, output_path: &PathBuf) -> Result<(PathBuf, f32), String> {
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
        
        let total_energy: f32 = samples.iter().map(|&x| x * x).sum();
        let total_rms = (total_energy / samples.len() as f32).sqrt();
        if total_rms < 0.003 {
            return Err("Audio was silent".to_string());
        }

        println!("[Typr] Captured {} raw samples", samples.len());

        let mono: Vec<f32> = if self.source_channels > 1 {
            samples
                .chunks(self.source_channels as usize)
                .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
                .collect()
        } else {
            samples.clone()
        };

        let mut resampled = resample(&mono, self.source_sample_rate, 16000);
        normalize_peak(&mut resampled, NORM_TARGET_PEAK, NORM_MAX_GAIN);
        println!("[Typr] Resampled to {} samples at 16kHz", resampled.len());

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
        self.samples.lock().unwrap().clear();

        println!("[Typr] WAV saved to {:?}", output_path);
        Ok((output_path.clone(), duration_secs))
    }
}

const NORM_TARGET_PEAK: f32 = 0.95;
const NORM_MAX_GAIN: f32 = 15.0;
const NORM_EPS: f32 = 1e-4;

/// Peak-normalize `samples` toward `target_peak`, but never amplify by more
/// than `max_gain` (so a near-silent clip's noise floor is not blown up).
/// Near-silent input is left unchanged.
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

fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return samples.to_vec();
    }

    let ratio = from_rate as f64 / to_rate as f64;
    let output_len = (samples.len() as f64 / ratio) as usize;
    let mut output = Vec::with_capacity(output_len);

    for i in 0..output_len {
        let src_idx = i as f64 * ratio;
        let idx = src_idx as usize;
        let frac = src_idx - idx as f64;

        let sample = if idx + 1 < samples.len() {
            samples[idx] as f64 * (1.0 - frac) + samples[idx + 1] as f64 * frac
        } else {
            samples[idx.min(samples.len() - 1)] as f64
        };

        output.push(sample as f32);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // A new higher block peak wins instantly.
        assert!((agc_update(0.5, 2.0, 0.995) - 2.0).abs() < 1e-6);
        // A quieter block decays the running peak gently (1.0 * 0.995).
        assert!((agc_update(1.0, 0.1, 0.995) - 0.995).abs() < 1e-6);
        // Never falls below the floor.
        assert!((agc_update(0.0, 0.0, 0.995) - 0.0001).abs() < 1e-9);
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
}
