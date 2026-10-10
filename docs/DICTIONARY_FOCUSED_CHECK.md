# Focused dictionary check

Restart the dev app so the rebuilt Rust backend is loaded. Use Notepad, AI cleanup **On**, GPT-OSS 120B, mode **Auto**. Keep these existing terms enabled: Glint, Tauri, Synara, Herdr, herdr-agent-handoff, Pi agent, CAT paper, Claude, terse and hyper.

The reported Nemotron/Parakeet recordings have already been replayed. Nemotron hit the old four-second AI deadline; Parakeet ran AI but recovery restored some valid multiword corrections. Both code paths are fixed and covered by regression tests. There is no need to repeat the whole checklist to diagnose those same logs. After restarting, one normal dictation can confirm the rebuilt app is in use; failures should still be reported with that capture's AI status line.

## Three recordings, about one minute total

Say the same passage once with **Nemotron**, once with **Parakeet v3**, and once with **local Whisper** if its model is already downloaded. Do not download another model just for this check.

> Open the Glint app and the Tauri repository. Ask Synara to review Herdr and the herdr-agent-handoff with Pi agent. Create study notes for my CAT paper. Ask the Claude assistant to keep this terse and check the hyper prefix.

Check:

- Saved spellings appear exactly, including CAT, Pi agent and herdr-agent-handoff.
- No requests or final words are dropped.
- Note seconds from stopping to paste for each engine.

**If a term fails:** paste the output and engine name. The existing STAGE 1 engine text, AI vocabulary length, AI model/retry/recovery line and final STAGE 6 identify whether it was misheard, omitted before cleanup, or rejected by cleanup. Only provide this capture's relevant log lines. A listed term cannot safely reconstruct a name that is completely absent from the ASR text.

## One optional context check (eight seconds)

With Nemotron and the same AI settings, say:

> There is a glint in her eye. Write a paper about my pet cat. Cloud computing is useful.

Expected: ordinary glint, pet cat and cloud computing stay ordinary; no Glint, CAT or Claude substitutions.

Whisper/Groq receive recognition hints with AI off. Current Nemotron/Parakeet decoders do not support dictionary hotwords, so use AI on for their contextual spelling check. Parakeet v3 chunk buffering is opt-in; it is not native streaming. These checks validate your recordings, not a guarantee that every future utterance is correct.
