//! Split a recording into decode-sized chunks at quiet points.
//!
//! Transducer models degrade on long audio. Measured on an 82-second dictation, Parakeet
//! silently dropped ~25 words — an entire clause — while the same audio cut to 27 seconds
//! transcribed it correctly. The model can hear the content; a single long decode loses it.
//!
//! So no decode ever sees more than `CHUNK_SECONDS`. Cutting on a fixed boundary would slice
//! through the middle of words, so the split point is the quietest moment near the end of the
//! window: real speech has pauses, and a pause is where a human would have cut too.
//!
//! Energy-based rather than a neural VAD, which keeps this dependency-free and unit-testable
//! without a model.

/// Longest audio handed to a single decode.
pub const CHUNK_SECONDS: f32 = 30.0;
/// How far back from the chunk limit to hunt for a pause.
const SPLIT_SEARCH_SECONDS: f32 = 5.0;
/// Width of the energy window scanned during that hunt.
const SPLIT_ENERGY_WINDOW_SECONDS: f32 = 0.1;

/// Split `samples` into chunks of at most `CHUNK_SECONDS`, cutting at quiet points.
///
/// Lossless and order-preserving: concatenating the result reproduces the input exactly.
/// Audio shorter than the limit is returned as a single chunk.
pub fn split_into_chunks(samples: &[f32], sample_rate: u32) -> Vec<&[f32]> {
    if samples.is_empty() {
        return Vec::new();
    }
    let limit = (CHUNK_SECONDS * sample_rate as f32) as usize;
    let search = (SPLIT_SEARCH_SECONDS * sample_rate as f32) as usize;
    let window = ((SPLIT_ENERGY_WINDOW_SECONDS * sample_rate as f32) as usize).max(1);

    let mut out = Vec::new();
    let mut rest = samples;
    while rest.len() > limit {
        let cut = quietest_split(rest, limit, search, window);
        let (head, tail) = rest.split_at(cut);
        out.push(head);
        rest = tail;
    }
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

/// Index of the lowest-energy point within the last `search` samples before `limit`.
///
/// Falls back to `limit` when no window fits, so the caller always makes progress and cannot
/// loop forever on a buffer with no quiet point.
fn quietest_split(samples: &[f32], limit: usize, search: usize, window: usize) -> usize {
    let limit = limit.min(samples.len());
    let start = limit.saturating_sub(search);
    let hop = (window / 2).max(1);

    let mut best_index = limit;
    let mut best_energy = f32::INFINITY;
    let mut pos = start;
    while pos + window <= limit {
        let energy: f32 = samples[pos..pos + window].iter().map(|s| s * s).sum();
        if energy < best_energy {
            best_energy = energy;
            best_index = pos + window / 2;
        }
        pos += hop;
    }
    best_index.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 16_000;

    /// Loud everywhere except a quiet valley at `quiet_at` seconds.
    fn buffer_with_quiet_at(total_secs: f32, quiet_at: f32) -> Vec<f32> {
        let n = (total_secs * SR as f32) as usize;
        let mut v = vec![0.5f32; n];
        let start = (quiet_at * SR as f32) as usize;
        let end = (start + SR as usize / 5).min(n); // 200ms of near-silence
        for s in v.iter_mut().take(end).skip(start) {
            *s = 0.0001;
        }
        v
    }

    #[test]
    fn test_short_audio_is_one_chunk() {
        let v = vec![0.5f32; (10.0 * SR as f32) as usize];
        let chunks = split_into_chunks(&v, SR);
        assert_eq!(chunks.len(), 1, "10s should not be split");
        assert_eq!(chunks[0].len(), v.len(), "no samples may be lost");
    }

    #[test]
    fn test_empty_input_yields_no_chunks() {
        assert!(split_into_chunks(&[], SR).is_empty());
    }

    /// The failure Task 1 measured: an 82s buffer must not reach the model in one piece.
    #[test]
    fn test_long_audio_is_split_and_loses_nothing() {
        let v = buffer_with_quiet_at(82.0, 28.0);
        let chunks = split_into_chunks(&v, SR);
        assert!(chunks.len() >= 3, "82s should yield 3+ chunks, got {}", chunks.len());
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, v.len(), "chunking must be lossless");
        for c in &chunks {
            let secs = c.len() as f32 / SR as f32;
            assert!(secs <= CHUNK_SECONDS + 0.01, "chunk of {:.1}s exceeds the limit", secs);
        }
    }

    /// The split must land in the quiet valley, not mid-word at the hard 30s boundary.
    #[test]
    fn test_split_prefers_the_quiet_point() {
        let v = buffer_with_quiet_at(60.0, 28.0);
        let chunks = split_into_chunks(&v, SR);
        let first_secs = chunks[0].len() as f32 / SR as f32;
        assert!(
            (first_secs - 28.1).abs() < 0.5,
            "expected the cut near the 28s quiet point, got {:.2}s",
            first_secs
        );
    }
}
