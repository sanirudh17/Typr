## [v0.2.1] - 2026-09-29 - Dictation Reliability & Accuracy Fixes

### 1. Dictation Stays Dictation
* AI post-processing now always returns your words as text. Dictated content is never answered, executed, or treated as a prompt, even when it contains instructions.
* Spoken commands, symbols, and keyboard shortcuts are formatted correctly instead of leaking into the text as raw words.

### 2. Capture Reliability
* Voice capture is more consistent across sessions. Silent audio and whispered end-of-session cues no longer end up in the pasted text, and real spoken content is never trimmed away.

### 3. Transcription Accuracy
* Improved handling of contractions, prepositions, and phonetic mishearings in everyday speech and technical terms.
* Dictionary entries keep their correct casing and no longer overwrite neighboring words.

### 4. Commands & Formatting
* List, layout, editing, casing, and symbol commands fire reliably in every mode while ordinary speech that merely sounds similar stays untouched.

### 5. Performance
* Cloud transcription of long dictations is noticeably faster.

### 6. Window & UI
* Close and maximize buttons work reliably, and the window always restores centered.
