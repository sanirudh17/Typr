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

#[derive(Clone, Debug)]
pub(crate) struct TimedWord {
    pub text: String,
    pub start: f32,
}

pub(crate) struct TimedChunkText {
    pub text: String,
    pub words: Option<Vec<TimedWord>>,
    pub overlaps_previous: bool,
}

fn timed_norm(word: &str) -> String {
    let normalized = norm(word);
    // Written/spoken ordinals refer to the same numbered item. Other digits
    // are never fuzzy-matched, so item 1 and item 2 remain distinct.
    match normalized.as_str() {
        "first" | "1st" => "1st".into(), "second" | "2nd" => "2nd".into(),
        "third" | "3rd" => "3rd".into(), "fourth" | "4th" => "4th".into(),
        "fifth" | "5th" => "5th".into(), "sixth" | "6th" => "6th".into(),
        "seventh" | "7th" => "7th".into(), "eighth" | "8th" => "8th".into(),
        "ninth" | "9th" => "9th".into(), "tenth" | "10th" => "10th".into(),
        _ => normalized,
    }
}

/// Reconcile corroborated words at the same audio time. A word is not dropped
/// merely because it occurs inside the replay window: unmatched words from
/// either decode survive. Unreliable/missing alignment uses conservative text
/// merging. No unconditional "trim first three seconds" is used.
pub(crate) fn merge_timed_chunks(parts: &[TimedChunkText]) -> String {
    let mut accumulated: Vec<TimedWord> = Vec::new();
    for part in parts {
        let Some(words) = &part.words else { return merge_chunk_texts(&fallback_and_rest(parts)); };
        let previous = accumulated.clone();
        let mut matched = vec![false; previous.len()];
        for (index, word) in words.iter().enumerate() {
            let same_time = |old: &TimedWord| (old.start - word.start).abs() <= 0.40;
            let same_word = |old: &TimedWord| timed_norm(&old.text) == timed_norm(&word.text);
            if part.overlaps_previous {
                if let Some(position) = previous.iter().enumerate().rev().take(24)
                    .find_map(|(position, old)| (!matched[position] && same_time(old) && same_word(old)).then_some(position)) {
                    matched[position] = true;
                    continue;
                }
            }
            // A different recognition of the first seam word (speech/Beach)
            // needs BOTH a time match and three exact following word anchors.
            // Keep the earlier, longer-left-context hypothesis in that case.
            let alternate = if part.overlaps_previous && index + 3 < words.len() && !seam_sensitive(&word.text) {
                previous.iter().enumerate().find_map(|(position, old)| {
                    (!matched[position] && same_time(old) && position + 3 < previous.len()
                        && !seam_sensitive(&old.text)
                        && (1..=3).all(|offset| {
                            let (before, after) = (&previous[position + offset], &words[index + offset]);
                            timed_norm(&before.text) == timed_norm(&after.text)
                                && (before.start - after.start).abs() <= 0.40
                        })).then_some(position)
                })
            } else { None };
            if let Some(position) = alternate { matched[position] = true; continue; }
            let position = accumulated.partition_point(|old| old.start <= word.start);
            accumulated.insert(position, word.clone());
        }
    }
    accumulated.iter().map(|word| word.text.as_str()).collect::<Vec<_>>().join(" ")
}

fn seam_sensitive(word: &str) -> bool {
    let word = timed_norm(word);
    word.chars().any(|ch| ch.is_ascii_digit()) || matches!(word.as_str(),
        "no" | "not" | "never" | "without" | "cannot" | "nothing" | "neither" | "nor"
        | "dont" | "cant" | "wont" | "isnt" | "arent" | "wasnt" | "werent"
        | "doesnt" | "didnt" | "shouldnt" | "wouldnt" | "couldnt" | "mustnt"
        | "hasnt" | "havent" | "hadnt")
}

fn fallback_and_rest(parts: &[TimedChunkText]) -> Vec<(String, bool)> {
    parts.iter().map(|part| (part.text.clone(), part.overlaps_previous)).collect()
}

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

/// The same boundary planner can be advanced while audio arrives. Only complete
/// 32 ms frames are inspected; unread frames and the final partial frame survive.
pub(crate) struct ChunkPlanner {
    frame: usize,
    force_cut: usize,
    arm: usize,
    overlap: usize,
    vad: AdaptiveVad,
    start: usize,
    pos: usize,
    overlaps_previous: bool,
    silence_run: usize,
    finished: bool,
}

impl ChunkPlanner {
    pub(crate) fn new(sample_rate: u32) -> Self {
        let sr = sample_rate.max(1) as f32;
        Self { frame: ((FRAME_MS / 1000.0) * sr) as usize,
            force_cut: (FORCE_CUT_SECS * sr) as usize,
            arm: (SILENCE_ARM_SECS * sr) as usize,
            overlap: (FORCED_OVERLAP_SECS * sr) as usize,
            vad: AdaptiveVad::new(), start: 0, pos: 0,
            overlaps_previous: false, silence_run: 0, finished: false }
    }

    pub(crate) fn advance(&mut self, samples: &[f32], finished: bool) -> Vec<(std::ops::Range<usize>, bool)> {
        if self.finished { return Vec::new(); }
        let mut out = Vec::new();
        if self.frame > 0 {
            while self.pos + self.frame <= samples.len() {
                let end = self.pos + self.frame;
                match self.vad.update(rms(&samples[self.pos..end])) {
                    VoiceActivity::Silent => self.silence_run += 1,
                    _ => self.silence_run = 0,
                }
                if (end - self.start >= self.arm
                    && self.silence_run >= (SILENCE_CUT_MS / FRAME_MS).ceil() as usize)
                    || end - self.start >= self.force_cut {
                    out.push((self.start..end, self.overlaps_previous));
                    self.overlaps_previous = true;
                    self.start = end.saturating_sub(self.overlap);
                    self.silence_run = 0;
                }
                self.pos = end;
            }
        }
        if finished {
            if self.start < samples.len() { out.push((self.start..samples.len(), self.overlaps_previous)); }
            self.finished = true;
        }
        out
    }
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
    if sample_rate == 0 {
        return vec![Chunk { samples, overlaps_previous: false }];
    }
    ChunkPlanner::new(sample_rate).advance(samples, true).into_iter()
        .map(|(range, overlaps_previous)| Chunk { samples: &samples[range], overlaps_previous }).collect()
}

/// How much of an overlapping chunk's start duplicates the previous chunk.
///
/// The engine needs this to know where to trim, so it is public rather than private to the
/// splitter.
pub const OVERLAP_SECS: f32 = FORCED_OVERLAP_SECS;

/// sherpa tokenizers mark a word start with U+2581 (lower one-eighth block).
const WORD_MARK: char = '\u{2581}';

/// Legacy timestamp-only trimmer, retained for compatibility, not used by the
/// production decoders. A previous decode may have omitted words inside its
/// final seconds, so blindly trimming the replay window can lose real speech.
///
/// Production merging corroborates words in both hypotheses and keeps unmatched
/// words; timestamps alone do not establish that a word was already recognized.
///
/// Falls back to the whole text when the model gives no timestamps or returns a count that
/// does not line up with the tokens, since a mismatched pair cannot be trusted to trim by.
pub fn trim_overlap_tokens(
    tokens: &[String],
    timestamps: Option<&[f32]>,
    fallback_text: &str,
    overlap_secs: f32,
) -> String {
    let Some(ts) = timestamps else {
        return fallback_text.trim().to_string();
    };
    if ts.len() != tokens.len() || overlap_secs <= 0.0 {
        return fallback_text.trim().to_string();
    }
    let kept: String = tokens
        .iter()
        .zip(ts.iter())
        .filter(|(_, t)| **t >= overlap_secs)
        .map(|(tok, _)| tok.as_str())
        .collect();
    kept.replace(WORD_MARK, " ").trim().to_string()
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
        let (replace, skip) = if *overlaps && !acc.is_empty() {
            overlap_len(&acc, &words)
        } else {
            (0, 0)
        };
        if replace > 0 {
            acc.truncate(acc.len() - replace);
            acc.extend(words[..skip].iter().map(|w| w.to_string()));
        }
        acc.extend(words[skip..].iter().map(|w| w.to_string()));
    }
    acc.join(" ")
}

/// Helper: encode 16 kHz mono f32 samples to in-memory WAV bytes.
pub fn samples_to_wav_bytes(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut cursor, spec)
        .map_err(|e| format!("Failed to create WAV writer: {}", e))?;
    for &s in samples {
        let sample_i16 = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        writer
            .write_sample(sample_i16)
            .map_err(|e| format!("Failed to write sample: {}", e))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("Failed to finalize WAV: {}", e))?;
    Ok(cursor.into_inner())
}

/// Helper: read samples from a WAV file path into f32 samples and sample rate.
pub fn read_wav_samples(path: &std::path::Path) -> Result<(Vec<f32>, u32), String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| format!("Failed to read audio file: {}", e))?;
    let sample_rate = reader.spec().sample_rate;
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.map(|v| v as f32 / 32768.0))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("Failed to decode audio samples: {}", e))?;
    Ok((samples, sample_rate))
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
    if seam_sensitive(&a) || seam_sensitive(&b) { return false; }
    // Numbers/digits must match exactly; e.g. "2" and "3" or "item 1" and "item 2" are distinct items.
    if a.chars().any(|c| c.is_ascii_digit()) || b.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    // Valid differing words are not spelling variants. The old majority rule
    // could drop "not", a different list number, or an adjective at the seam.
    if crate::vocab_correct::is_common_word(&a) && crate::vocab_correct::is_common_word(&b) {
        return false;
    }
    let budget = if a.len().max(b.len()) <= 4 { 1 } else { 2 };
    edit_distance(&a, &b) <= budget
}

/// Longest prefix of `new` that repeats the tail of `acc`, capped at `MERGE_WINDOW_WORDS`.
///
/// Every skipped word must be corroborated. A richer overlapping decode may
/// replace a shorter tail only when it contains every word of that tail in
/// order, with both ends anchored. Unique newly recognized words survive.
fn overlap_len(acc: &[String], new: &[&str]) -> (usize, usize) {
    let max = MERGE_WINDOW_WORDS.min(acc.len()).min(new.len());
    for n in (1..=max).rev() {
        let tail = &acc[acc.len() - n..];
        if tail.iter().zip(new).all(|(a, b)| words_similar(a, b)) {
            return (0, n);
        }
        if n < 3 { continue; }
        for m in (3..=MERGE_WINDOW_WORDS.min(new.len())).rev() {
            let prefix = &new[..m];
            if !words_similar(&tail[0], prefix[0]) || !words_similar(&tail[n - 1], prefix[m - 1]) { continue; }
            let is_subsequence = |short: &[&str], long: &[&str]| {
                let mut index = 0;
                for word in long {
                    if index < short.len() && words_similar(short[index], word) { index += 1; }
                }
                index == short.len()
            };
            let old: Vec<_> = tail.iter().map(String::as_str).collect();
            if m > n && is_subsequence(&old, prefix) { return (n, m); }
            if n > m && is_subsequence(prefix, &old) { return (0, m); }
        }
    }
    (0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 16_000;

    fn timed(text: &str, starts: &[f32], overlaps_previous: bool) -> TimedChunkText {
        let words: Vec<_> = text.split_whitespace().zip(starts).map(|(text, &start)|
            TimedWord { text: text.into(), start }).collect();
        assert_eq!(words.len(), text.split_whitespace().count());
        TimedChunkText { text: text.into(), words: Some(words), overlaps_previous }
    }

    #[test]
    fn timed_seams_remove_corroborated_duplicates_and_retain_new_words() {
        assert_eq!(merge_timed_chunks(&[
            timed("review speech engines before deployment.", &[19., 20., 21., 22., 23.], false),
            timed("Beach engines before deployment. Continue.", &[20.08, 21.08, 22.08, 23.08, 24.], true),
        ]), "review speech engines before deployment. Continue.");
        assert_eq!(merge_timed_chunks(&[
            timed("carried consecutive chunks", &[0., 0.8, 1.2], false),
            timed("carried between consecutive chunks and finish", &[0., 0.4, 0.8, 1.2, 1.6, 2.], true),
        ]), "carried between consecutive chunks and finish");
    }

    #[test]
    fn timed_seams_do_not_collapse_real_repetitions() {
        assert_eq!(merge_timed_chunks(&[
            timed("very", &[0.], false),
            timed("very very good", &[0., 0.3, 0.6], true),
        ]), "very very good");
        assert_eq!(merge_timed_chunks(&[timed("very very good", &[0., 0.2, 0.4], false)]), "very very good");
        assert_eq!(merge_timed_chunks(&[
            timed("again", &[0.], false), timed("again", &[1.], true),
        ]), "again again");
    }

    #[test]
    fn timed_seams_keep_numbers_negations_and_unaligned_text() {
        let text = merge_timed_chunks(&[
            timed("do work without AI", &[0., 1., 2., 3.], false),
            timed("don't work without AI", &[0., 1., 2., 3.], true),
        ]);
        assert!(text.contains("don't"), "a changed negation must not be skipped");
        assert_eq!(merge_timed_chunks(&[
            timed("item 1", &[0., 1.], false), timed("item 2", &[0., 1.], true),
        ]), "item 1 2");
        assert_eq!(merge_timed_chunks(&[
            timed("keep final words", &[0., 1., 2.], false),
            TimedChunkText { text: "keep all final words".into(), words: None, overlaps_previous: true },
        ]), "keep all final words");
        assert!(!words_similar("mustn't", "must"));
        assert!(!words_similar("not", "tot"));
    }

    #[test]
    fn incremental_boundaries_match_batch_and_cover_final_partial_frame() {
        for input in [speech(79.887), speech_with_pause(44.053, 20.0, 2.0)] {
            let expected = ChunkPlanner::new(SR).advance(&input, true);
            for size in [1, 511, 2400, 17001] {
                let mut planner = ChunkPlanner::new(SR);
                let mut actual = Vec::new();
                for end in (size..input.len()).step_by(size) { actual.extend(planner.advance(&input[..end], false)); }
                actual.extend(planner.advance(&input, true));
                assert_eq!(actual, expected);
                assert_eq!(actual.last().unwrap().0.end, input.len());
                assert!(planner.advance(&input, true).is_empty(), "tail must flush once");
                for pair in actual.windows(2) { assert!(pair[1].0.start <= pair[0].0.end); }
            }
        }
    }

    #[test]
    fn seams_preserve_new_words_negations_and_distinct_numbers() {
        for (before, after, expected) in [
            ("states are preserved and carried consecutive chunks", "carried between consecutive chunks then continue",
             "states are preserved and carried between consecutive chunks then continue"),
            ("keep all the final words", "keep all final words and finish", "keep all the final words and finish"),
            ("the model does work", "the model does not work", "the model does not work"),
            ("check item 1", "check item 2 and finish", "check item 1 check item 2 and finish"),
            ("keep the final word", "keep the final work safe", "keep the final word keep the final work safe"),
        ] {
            assert_eq!(merge_chunk_texts(&[(before.into(), false), (after.into(), true)]), expected);
        }
    }

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
    fn test_trim_drops_tokens_inside_the_overlap() {
        let tokens: Vec<String> =
            ["\u{2581}old", "\u{2581}words", "\u{2581}new", "\u{2581}words"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        // First two were spoken inside the 3s replayed window; last two after it.
        let ts = [0.5f32, 1.8, 3.4, 4.0];
        assert_eq!(trim_overlap_tokens(&tokens, Some(&ts), "ignored", 3.0), "new words");
    }

    #[test]
    fn test_trim_falls_back_when_timestamps_are_absent() {
        let tokens = vec!["\u{2581}anything".to_string()];
        assert_eq!(trim_overlap_tokens(&tokens, None, "the full text", 3.0), "the full text");
    }

    /// A count mismatch means the pairing is unreliable, so trimming by it could delete real
    /// speech. Returning everything risks a duplicate, which is visible and harmless by
    /// comparison.
    #[test]
    fn test_trim_falls_back_when_counts_disagree() {
        let tokens = vec!["\u{2581}a".to_string(), "\u{2581}b".to_string()];
        let ts = [0.1f32];
        assert_eq!(trim_overlap_tokens(&tokens, Some(&ts), "the full text", 3.0), "the full text");
    }

    #[test]
    fn test_trim_is_a_no_op_without_overlap() {
        let tokens = vec!["\u{2581}kept".to_string()];
        let ts = [0.1f32];
        assert_eq!(trim_overlap_tokens(&tokens, Some(&ts), "kept", 0.0), "kept");
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

    /// Simulating a decoder that prematurely dropped words at the chunk boundary:
    /// Chunk 1 decode died mid-sentence: "Let me know whether there are"
    /// Chunk 2 started inside the 3s overlap and decoded: "whether there are some changes that you would like to make."
    /// The merged result must recover 100% of the speech without duplicating the seam.
    #[test]
    fn test_merge_recovers_tail_dropping_decode() {
        let parts = vec![
            ("Item 1. Item 2. Item 3. Let me know whether there are".to_string(), false),
            ("whether there are some changes that you would like to make.".to_string(), true),
        ];
        assert_eq!(
            merge_chunk_texts(&parts),
            "Item 1. Item 2. Item 3. Let me know whether there are some changes that you would like to make."
        );
    }
}
