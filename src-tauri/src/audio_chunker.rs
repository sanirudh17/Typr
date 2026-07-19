//! Split a recording into decode-sized chunks at real speech boundaries.
//!
//! Transducer models degrade on long audio. Measured on an 82-second dictation, Parakeet
//! silently dropped ~25 words — an entire clause — while the same audio cut to 27 seconds
//! transcribed it correctly. The model can hear the content; one long decode loses it.
//!
//! The first attempt cut at the quietest 100ms window before a fixed 30s boundary. That
//! recovered about half the lost clause and no more: on continuous speech the "quietest"
//! moment is still inside a word, so words died at every seam, and with no overlap there was
//! nothing to recover them from.
//!
//! This version cuts where the speaker actually stopped:
//!
//! - An adaptive noise floor learns what silence sounds like for this microphone and room,
//!   with separate speech/silence thresholds so the detector cannot flap mid-word.
//! - A cut happens after a genuine run of silence, and only once the chunk is long enough to
//!   be worth cutting — short pauses inside a sentence must not fragment it.
//! - If a speaker never pauses, a forced cut lands at the hard limit and the next chunk
//!   **overlaps** it, so a word split across the boundary appears in full in one of them.
//! - Overlapping text is then reconciled by `merge_chunk_texts`.
//!
//! Pure arithmetic over the sample buffer: no model, no dependency, fully unit-testable.

// ── Adaptive voice-activity detection ───────────────────────────────────────
const FLOOR_INIT: f32 = 0.003;
/// The floor drops quickly, so a quiet room is recognised fast.
const FLOOR_ALPHA_DOWN: f32 = 0.2;
/// It rises slowly, so sustained speech cannot drag the floor up behind it.
const FLOOR_ALPHA_UP: f32 = 0.005;
/// Loud frames are excluded from the rising average entirely — that is speech, not noise.
const FLOOR_UP_MAX_RMS_RATIO: f32 = 10.0;
const K_SPEECH: f32 = 5.0;
const K_SILENCE: f32 = 3.0;
const SPEECH_THRESHOLD_MIN: f32 = 0.004;
const SPEECH_THRESHOLD_MAX: f32 = 0.08;
const VAD_EMA_ALPHA: f32 = 0.3;

// ── Chunking policy ─────────────────────────────────────────────────────────
/// Frame granularity for the energy scan.
const FRAME_MS: f32 = 32.0;
/// A chunk must reach this length before a silence is allowed to end it, or ordinary
/// sentence pauses would shatter a dictation into fragments.
const SILENCE_ARM_SECS: f32 = 12.0;
/// How much continuous silence counts as "the speaker stopped".
const SILENCE_CUT_MS: f32 = 900.0;
/// Hard ceiling. Reached only when someone talks for a minute without pausing.
const FORCE_CUT_SECS: f32 = 25.0;
/// A forced cut lands mid-speech, so the next chunk starts this far back.
const FORCED_OVERLAP_SECS: f32 = 3.0;
/// Words compared when reconciling an overlapped seam.
const MERGE_WINDOW_WORDS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq)]
enum VoiceActivity {
    /// No speech yet observed — still calibrating on background noise.
    NotStarted,
    Active,
    Silent,
}

struct AdaptiveVad {
    noise_floor: f32,
    smoothed_rms: f32,
    has_speech_started: bool,
}

impl AdaptiveVad {
    fn new() -> Self {
        Self { noise_floor: FLOOR_INIT, smoothed_rms: 0.0, has_speech_started: false }
    }

    fn observe(&mut self, rms: f32) {
        if rms < self.noise_floor {
            self.noise_floor = FLOOR_ALPHA_DOWN * rms + (1.0 - FLOOR_ALPHA_DOWN) * self.noise_floor;
        } else {
            let floor_base = self.noise_floor.max(SPEECH_THRESHOLD_MIN / K_SPEECH);
            if rms <= floor_base * FLOOR_UP_MAX_RMS_RATIO {
                self.noise_floor = FLOOR_ALPHA_UP * rms + (1.0 - FLOOR_ALPHA_UP) * self.noise_floor;
            }
        }
        self.smoothed_rms = VAD_EMA_ALPHA * rms + (1.0 - VAD_EMA_ALPHA) * self.smoothed_rms;
    }

    fn speech_threshold(&self) -> f32 {
        (self.noise_floor * K_SPEECH).clamp(SPEECH_THRESHOLD_MIN, SPEECH_THRESHOLD_MAX)
    }

    /// Lower than the speech threshold on purpose: the gap between the two is hysteresis, and
    /// without it a detector oscillates across a single threshold in the middle of a word.
    fn silence_threshold(&self) -> f32 {
        (self.noise_floor * K_SILENCE)
            .clamp(SPEECH_THRESHOLD_MIN * 0.6, SPEECH_THRESHOLD_MAX * 0.6)
    }

    fn update(&mut self, rms: f32) -> VoiceActivity {
        self.observe(rms);
        if self.smoothed_rms > self.speech_threshold() {
            self.has_speech_started = true;
        }
        if !self.has_speech_started {
            VoiceActivity::NotStarted
        } else if self.smoothed_rms < self.silence_threshold() {
            VoiceActivity::Silent
        } else {
            VoiceActivity::Active
        }
    }
}

/// One decode-sized slice of the recording.
#[derive(Debug, Clone, Copy)]
pub struct Chunk<'a> {
    pub samples: &'a [f32],
    /// True when this chunk replays the tail of the previous one, so its transcript will
    /// restate words already seen. `merge_chunk_texts` uses this to know when to de-duplicate.
    pub overlaps_previous: bool,
}

fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

/// Split `samples` at speech boundaries, forcing a cut if the speaker never pauses.
///
/// Audio shorter than the forced-cut ceiling with no qualifying silence comes back as a single
/// chunk. Every sample appears in at least one chunk; overlapped regions appear in two.
pub fn split_into_chunks(samples: &[f32], sample_rate: u32) -> Vec<Chunk<'_>> {
    if samples.is_empty() {
        return Vec::new();
    }
    let sr = sample_rate as f32;
    let frame = ((FRAME_MS / 1000.0) * sr) as usize;
    let force_cut = (FORCE_CUT_SECS * sr) as usize;
    let arm = (SILENCE_ARM_SECS * sr) as usize;
    let overlap = (FORCED_OVERLAP_SECS * sr) as usize;
    let silence_frames_needed = ((SILENCE_CUT_MS / FRAME_MS).ceil() as usize).max(1);

    if frame == 0 || samples.len() <= arm {
        return vec![Chunk { samples, overlaps_previous: false }];
    }

    let mut out = Vec::new();
    let mut vad = AdaptiveVad::new();
    let mut start = 0usize;
    let mut overlaps_previous = false;
    let mut silence_run = 0usize;
    let mut pos = 0usize;

    while pos + frame <= samples.len() {
        let end = pos + frame;
        match vad.update(rms(&samples[pos..end])) {
            VoiceActivity::Silent => silence_run += 1,
            _ => silence_run = 0,
        }

        let long_enough = end - start >= arm;
        let paused = silence_run >= silence_frames_needed;
        let forced = end - start >= force_cut;

        if (long_enough && paused) || forced {
            out.push(Chunk { samples: &samples[start..end], overlaps_previous });
            // A pause is a clean boundary and needs no overlap; a forced cut lands mid-word,
            // so the next chunk rewinds to catch whatever was severed.
            overlaps_previous = true;
            start = end.saturating_sub(overlap);
            silence_run = 0;
        }
        pos = end;
    }

    if start < samples.len() {
        out.push(Chunk { samples: &samples[start..], overlaps_previous });
    }
    out
}

/// Join per-chunk transcripts, removing text an overlapped chunk restates.
///
/// The overlap is an audio window, not a word count, so the repeated text is found rather than
/// assumed: the longest suffix of what we have that matches a prefix of what arrived, up to
/// `MERGE_WINDOW_WORDS`. Comparison is case- and punctuation-insensitive because the decoder
/// re-punctuates each chunk independently and would otherwise never match.
pub fn merge_chunk_texts(parts: &[(String, bool)]) -> String {
    let mut acc: Vec<String> = Vec::new();
    for (text, overlaps) in parts {
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        let skip = if *overlaps && !acc.is_empty() {
            overlap_len(&acc, &words)
        } else {
            0
        };
        acc.extend(words[skip..].iter().map(|w| w.to_string()));
    }
    acc.join(" ")
}

fn norm(w: &str) -> String {
    w.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// Levenshtein distance, two-row variant. Small inputs only — these are single words.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Whether two words are the same word as far as a seam is concerned.
///
/// The two chunks decoded the same audio separately, so the overlap is rarely identical:
/// measured seams produced "pros"/"prose", "add"/"ad", and "preemptive"/"preem to". Exact
/// comparison misses all of those and leaves the text duplicated, so a small edit budget
/// applies — tighter on short words, where one edit is a larger proportional change.
fn words_similar(a: &str, b: &str) -> bool {
    let (a, b) = (norm(a), norm(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    let budget = if a.len().max(b.len()) <= 4 { 1 } else { 2 };
    edit_distance(&a, &b) <= budget
}

/// Longest prefix of `new` that repeats the tail of `acc`, capped at `MERGE_WINDOW_WORDS`.
///
/// A seam is accepted on a majority of similar words rather than all of them: across three
/// seconds of re-decoded audio a couple of words routinely differ beyond an edit or two, and
/// demanding a perfect run means never matching at all. The first word must still align, which
/// anchors the seam and stops a chance mid-sentence resemblance from eating real text.
fn overlap_len(acc: &[String], new: &[&str]) -> usize {
    let max = MERGE_WINDOW_WORDS.min(acc.len()).min(new.len());
    for n in (1..=max).rev() {
        let tail = &acc[acc.len() - n..];
        if !words_similar(&tail[0], new[0]) {
            continue;
        }
        let matches = tail.iter().zip(new.iter()).filter(|(a, b)| words_similar(a, b)).count();
        let needed = if n <= 2 { n } else { (n * 2).div_ceil(3) };
        if matches >= needed {
            return n;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 16_000;

    fn secs(n: f32) -> usize {
        (n * SR as f32) as usize
    }

    /// Continuous speech-level audio: loud enough to clear the speech threshold everywhere.
    fn speech(total_secs: f32) -> Vec<f32> {
        vec![0.3f32; secs(total_secs)]
    }

    /// Speech with a genuine silent gap of `gap_secs` starting at `at_secs`.
    fn speech_with_pause(total_secs: f32, at_secs: f32, gap_secs: f32) -> Vec<f32> {
        let mut v = speech(total_secs);
        let a = secs(at_secs);
        let b = (a + secs(gap_secs)).min(v.len());
        for s in v.iter_mut().take(b).skip(a) {
            *s = 0.0;
        }
        v
    }

    #[test]
    fn test_empty_input_yields_no_chunks() {
        assert!(split_into_chunks(&[], SR).is_empty());
    }

    #[test]
    fn test_short_audio_is_one_chunk() {
        let v = speech(10.0);
        let chunks = split_into_chunks(&v, SR);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].samples.len(), v.len());
        assert!(!chunks[0].overlaps_previous);
    }

    /// A pause inside a sentence must not fragment the dictation.
    #[test]
    fn test_early_pause_does_not_cut() {
        let v = speech_with_pause(20.0, 5.0, 1.0);
        let chunks = split_into_chunks(&v, SR);
        assert_eq!(chunks.len(), 1, "a pause before the arming point must not cut");
    }

    /// The behaviour this module exists for: cut where the speaker actually stopped.
    /// The gap is 2s, not 1s, and that is not arbitrary. The detector smooths its input, so
    /// roughly 300ms of a pause elapses before it reports silence at all; a gap must exceed
    /// that lag plus `SILENCE_CUT_MS` to count. A 1s gap does not, and should not — measured
    /// on real dictation, the gaps between spoken list items are about that long, and cutting
    /// there is what shredded a seven-item list across three chunks.
    #[test]
    fn test_cuts_at_a_real_pause_after_arming() {
        let v = speech_with_pause(40.0, 20.0, 2.0);
        let chunks = split_into_chunks(&v, SR);
        assert!(chunks.len() >= 2, "expected a cut at the 20s pause, got {}", chunks.len());
        let first = chunks[0].samples.len() as f32 / SR as f32;
        assert!(
            (18.0..24.0).contains(&first),
            "cut should land near the pause, got {:.1}s",
            first
        );
        // Every seam overlaps, including one at a pause. Measured on real dictation: even
        // cutting at a genuine pause, the decoder loses its first words after a reset, so the
        // next chunk always rewinds. A pause is a better place to cut, not a safe one.
        assert!(chunks[1].overlaps_previous, "every seam must overlap");
    }

    /// Someone who never pauses still must not hand 90 seconds to one decode.
    #[test]
    fn test_unbroken_speech_is_force_cut_with_overlap() {
        let v = speech(90.0);
        let chunks = split_into_chunks(&v, SR);
        assert!(chunks.len() >= 2, "90s of unbroken speech must be split");
        assert!(chunks[1].overlaps_previous, "a forced cut must overlap the next chunk");
        let total: usize = chunks.iter().map(|c| c.samples.len()).sum();
        assert!(total > v.len(), "overlap means chunks cover more than the input");
    }

    #[test]
    fn test_merge_plain_chunks_just_joins() {
        let parts = vec![("hello world".to_string(), false), ("second part".to_string(), false)];
        assert_eq!(merge_chunk_texts(&parts), "hello world second part");
    }

    #[test]
    fn test_merge_removes_overlapped_words() {
        let parts = vec![
            ("the quick brown fox".to_string(), false),
            ("brown fox jumps over".to_string(), true),
        ];
        assert_eq!(merge_chunk_texts(&parts), "the quick brown fox jumps over");
    }

    /// The decoder re-punctuates and re-capitalises each chunk, so a literal match would fail.
    #[test]
    fn test_merge_ignores_case_and_punctuation_at_the_seam() {
        let parts = vec![
            ("we discussed the budget".to_string(), false),
            ("The budget, again".to_string(), true),
        ];
        assert_eq!(merge_chunk_texts(&parts), "we discussed the budget again");
    }

    #[test]
    fn test_merge_keeps_everything_when_nothing_repeats() {
        let parts =
            vec![("alpha beta".to_string(), false), ("gamma delta".to_string(), true)];
        assert_eq!(merge_chunk_texts(&parts), "alpha beta gamma delta");
    }
}
