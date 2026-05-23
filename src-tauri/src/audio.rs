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

struct SendStream(#[allow(dead_code)] cpal::Stream);
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}

pub struct AudioRecorder {
    samples: Arc<Mutex<Vec<f32>>>,
    stream: Option<SendStream>,
    active_mic: Option<String>,
    source_sample_rate: u32,
    source_channels: u16,
    amplitude_ring: Arc<Mutex<Vec<f32>>>,
    amplitude_index: Arc<Mutex<usize>>,
    fft: StdArc<dyn Fft<f32>>,
    fft_buffer: Arc<Mutex<Vec<Complex<f32>>>>,
    frequency_bands: Arc<Mutex<Vec<f32>>>,
    fft_callback_divider: Arc<Mutex<u8>>,
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
            active_mic: None,
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

    pub fn ensure_initialized(&mut self, mic_name: &str) -> Result<(), String> {
        if self.stream.is_some() && self.active_mic.as_deref() == Some(mic_name) {
            return Ok(());
        }

        let host = cpal::default_host();

        let device = if mic_name == "default" {
            host.default_input_device()
                .ok_or("No default input device found")?
        } else {
            host.input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().map(|n| n == mic_name).unwrap_or(false))
                .ok_or(format!("Microphone '{}' not found", mic_name))?
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
                |err| {
                    eprintln!("[Typr] Audio stream error: {}", err);
                },
                None,
            )
            .map_err(|e| e.to_string())?;

        let _ = stream.pause();
        self.stream = Some(SendStream(stream));
        self.active_mic = Some(mic_name.to_string());
        println!("[Typr] Audio stream pre-initialized and paused for microphone '{}'", mic_name);
        Ok(())
    }

    pub fn start(&mut self, mic_name: &str) -> Result<(), String> {
        self.ensure_initialized(mic_name)?;

        self.samples.lock().unwrap().clear();
        {
            let mut ring = self.amplitude_ring.lock().unwrap();
            for v in ring.iter_mut() {
                *v = 0.0;
            }
        }
        *self.amplitude_index.lock().unwrap() = 0;

        if let Some(ref s) = self.stream {
            s.0.play().map_err(|e| e.to_string())?;
        }
        println!("[Typr] Audio recording started");
        Ok(())
    }

    pub fn stop_and_save(&mut self, output_path: &PathBuf) -> Result<(PathBuf, f32), String> {
        if let Some(ref s) = self.stream {
            let _ = s.0.pause();
        }
        println!("[Typr] Audio recording paused");

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

        let resampled = resample(&mono, self.source_sample_rate, 16000);
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
