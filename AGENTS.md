# Typr — Agent Reference

This file is for future coding agents (and humans) working on Typr. It captures invariants that have caused regressions.

## 1. Window must launch centered — every time

**Requirement:** Fresh launch *and* re-show from tray/background must appear in the middle of the screen. No top-left drift.

**Implementation (do not remove):**
- `src-tauri/src/main.rs` setup builds the `main` window dynamically via
  `WebviewWindowBuilder` (1160×720, min 700×500, frameless, `center()`,
  `visible(false)` + explicit show) — NOT a static `tauri.conf.json` entry,
  so the native `background_color` and the `__TYPR_BOOT__` init script can be
  seeded per-launch from settings (this is what kills the cold-start theme
  flash; a post-hoc `set_background_color` races first show and loses)
- `src-tauri/src/main.rs` → explicit `window.center()` before `window.show()` in:
  1. `setup` fresh launch (non-`--hidden` path)
  2. tray `on_menu_event("show")`
  3. `on_tray_icon_event` left-click
- `src-tauri/src/main.rs` → `try_focus_existing_window()` for the second-process case (single-instance mutex `Local\TyprSingleInstanceMutex`). Second launch of `Typr.exe` while hidden to tray must find the existing `"Typr"` HWND via `FindWindowW`, `ShowWindow(SW_RESTORE)`, `SetWindowPos` centered at `(screen_w-1160)/2, (screen_h-720)/2`, `SetForegroundWindow`. Otherwise the user sees "nothing happened".

If you change window size, update both `tauri.conf.json` and the `win_w/win_h` constants in `try_focus_existing_window`.

## 2. AI prompts — anti-quote / anti-camelCase / intelligent layout

All LLM prompts in `src-tauri/src/ai_postprocess.rs` must:

- `NEVER add quotation marks` around ordinary phrases and `NEVER join` separate words into camelCase/PascalCase/snake_case unless the user explicitly said a casing command (`"camel case ..."`) or dictated a path with separators (`"src slash main dot rs"`). Regression reference: `"quick overlay button"` → must stay 3 words, never `"quick overlay button"` or `quickOverlayButton`.
- Format intelligently and automatically: long/multi-topic → short paragraphs (blank line), enumeration → `"- "` bullets. Do not force single paragraph, do not add bullets/headings to single thought. Each Auto profile (Messaging, Email, Professional, Developer) and `CLEANUP_PROMPT` + `PROMPT_MODE_NATURAL` carries its own variant — see tests `test_cleanup_prompt_guards_*`, `test_developer_prompt_guards_*`, `test_auto_prompts_all_guard_*`.
- Preserve `NEVER_REFUSE_CLAUSE` ordering: `build_system_prompt(base, tone, format, custom)` = base + `NEVER_REFUSE_CLAUSE` + style suffix, contract before style.

Tests: `cargo test --lib` must stay at 267+ passing (263 original + 4 new AI prompt guards). Do not weaken guards to make a prompt "shorter".

## 3. Native `<select>` dropdowns are banned

All `<select>` elements must use the custom dark panel that matches `.app-picker-panel` (`src/style.css` → `.custom-select*`). Native Windows grey dropdown is a regression (see GH screenshots).

- `src/main.ts` → `enhanceSelect()` / `initCustomSelects()` wraps every `select` (mic, Whisper model, Parakeet model, AI model, `auto-context-override`, `app-rule-category`). Native element is `display:none` source of truth; trigger + `position:fixed` panel appended to `body`.
- After any programmatic `select.value = ...` call `syncCustomSelectDisplay(select)` (or `MutationObserver` will miss it). Already wired in `populateMics`, `setParakeetModel`, `setAiModel`, `loadSettings`.
- Panel positioning via `positionCustomPanel()` with flip-above logic; `closeAllCustomSelects()` on `mousedown` outside, `Escape`, `resize`, `scroll`.

## 4. Checking changes

- TypeScript: `C:\Users\sanir\Typr\node_modules\.bin\tsc.cmd --noEmit --project tsconfig.json` (or `npx tsc`) — must be clean.
- Frontend: `vite build` — must succeed (catches `style.css` stray `}`).
- Rust: `cargo test --lib` (needs dummy `src-tauri/binaries/*.dll` + `whisper-*.exe` placeholders if `tauri.conf.json` resources are missing). Current baseline: 267 passed.
- Do not edit `package.json` manually for deps — use install command.
