## [v0.2.2] - 2026-10-06 - Nemotron Streaming Accuracy, Visible Failure Feedback & Command Robustness

### 1. Nemotron Streaming Speed & Completeness
* **Single-Pass Short Dictations:** Clips up to 8 seconds now decode once instead of live-then-verify, restoring instant paste while removing the live chunk-boundary seam that dropped final words.
* **Sparse-Stream Verification:** Long dictations keep true live streaming; if the live transcript comes back suspiciously thin, a batch pass automatically recovers the richer transcript.

### 2. Capture Reliability & Diagnostics
* **Complete Tail Drain:** The live session fully drains captured audio before the capture buffer is released, so final words are never cut at the source; clip energy is measured on the raw capture before gain is applied.
* **End-of-Capture Diagnostics:** Tail-energy readings distinguish audio cut at the microphone from words dropped by the engine, making truncation reports directly actionable.

### 3. Failure Feedback You Can See
* **Quiet-Tap Guard:** A lone engine token from a multi-second capture is recognized as engine noise and never pasted; too-short, silent, and empty captures log the reason instead of failing silently.
* **Visible Errors Everywhere:** Failures surface in-app when the window is open and as a system notification when dictating from the tray, so a missed dictation always explains itself.

### 4. Voice Command Robustness
* **Pronoun-Drop Tolerance:** "Make a list" and "make this a list" fire the list command even when the engine swallows the pronoun, with a prose gate so ordinary speech such as "make a list of groceries" stays literal.
* **Last-Sentence Listing & Homophones:** The list command targets the final sentence after a terminator, and the frequent "new line" mis-transcription is accepted with article-aware prose gating.

### 5. Dictionary & Cleanup Precision
* **Fragment Guard:** Dictionary hints no longer rewrite word fragments, ending corruptions of short ordinary words into product terms while genuine technical terms still correct normally.
* **Case-Preserving Cue Stripping:** Trailing stop phrases strip without lowercasing names, and fixed native input gain replaces the manual gain slider for consistent capture levels.

### 6. AI Repair & Formatting Grounding
* **Contextual Garble Repair:** Repair prompts now cover multi-word appearance garbles and shopping-list slips, applied only when the literal reading makes no sense in context — ordinary words are never rewritten into technical terms.
* **Consistent Formatting:** Bullet rules are unified across profiles, numbered enumerations format cleanly, and divergence checks keep AI output grounded to what was actually dictated.

### 7. Terminal & Developer Context Fixes
* **Literal Command Preservation:** Lone command-like phrases keep their exact form in terminal context, and prose dictated inside a terminal keeps normal sentence formatting instead of being inferred into commands.
* **Tuning Escape Hatches:** Batch verification can be switched off for pure live-streaming speed, and a compare mode logs live-versus-batch transcripts per dictation for accuracy diagnosis.
