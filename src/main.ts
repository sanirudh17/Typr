import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { save } from "@tauri-apps/plugin-dialog";
import { check as checkForUpdate, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";

interface Settings {
  microphone: string;
  engine: string;
  whisperModel: string;
  groqApiKey: string;
  recordingMode: string;
  hotkey: string;
  cloudModel: string;
  parakeetModel: string;
  aiEnabled: boolean;
  aiModel: string;
  aiProfile: string;
  aiPromptFormat: string;
  aiTone: string;
  aiFormat: string;
  aiCustomInstructions: string;
  backgroundMode: boolean;
  autostart: boolean;
  hotkeySecondary: string;
  secondaryProfile: string;
  autoContextOverride: string;
  appRules: AppRule[];
}

interface AppRule {
  process_name: string;
  title_contains: string | null;
  category: string;
}

interface RunningApp {
  process_name: string;
  display_name: string;
}

interface TranscriptionItem {
  id: string;
  timestamp: number;
  text: string;
  duration_secs: number;
  word_count: number;
}

interface History {
  items: TranscriptionItem[];
}

interface MicDevice {
  name: string;
  is_default: boolean;
}

interface DownloadProgress {
  downloaded: number;
  total: number;
  percent: number;
}

// DOM elements
const statusDot = document.getElementById("status-dot")!;
const statusIndicator = document.getElementById("status-indicator")!;
const statusText = document.getElementById("status-text")!;
const micSelect = document.getElementById("mic-select") as HTMLSelectElement;
const engineLocal = document.getElementById("engine-local")!;
const engineCloud = document.getElementById("engine-cloud")!;
const engineParakeet = document.getElementById("engine-parakeet")!;
const parakeetSettings = document.getElementById("parakeet-settings")!;
const parakeetModelSelect = document.getElementById("parakeet-model-select") as HTMLSelectElement;
const parakeetDownloadBtn = document.getElementById("parakeet-download-btn") as HTMLButtonElement;
const localSettings = document.getElementById("local-settings")!;
const cloudSettings = document.getElementById("cloud-settings")!;
const modelSelect = document.getElementById("model-select") as HTMLSelectElement;
const downloadBtn = document.getElementById("download-btn")!;
const downloadProgress = document.getElementById("download-progress")!;
const progressFill = document.getElementById("progress-fill")!;
const parakeetDownloadProgress = document.getElementById("parakeet-download-progress")!;
const parakeetProgressFill = document.getElementById("parakeet-progress-fill")!;
// Each engine owns its own progress bar (a single shared one jumped position when switching
// tabs mid-download). A download is in flight only for the engine whose flag is set; the
// progress event routes to whichever fill is active.
let whisperDownloading = false;
let parakeetDownloading = false;
let activeProgressFill: HTMLElement | null = null;
const groqKey = document.getElementById("groq-key") as HTMLInputElement;
// The same key powers two different features on two different tabs: Cloud transcription
// (Engine) and AI cleanup (AI). It used to live only under the Engine tab's Cloud block,
// which is hidden unless Cloud is the selected engine — so anyone on Local Whisper or
// Parakeet could turn AI cleanup on and have nowhere to enter the key it needs. Both inputs
// edit one setting and are mirrored on every keystroke.
const aiGroqKey = document.getElementById("ai-groq-key") as HTMLInputElement;
const aiKeyMissingNote = document.getElementById("ai-key-missing-note")!;
const engineSetupNote = document.getElementById("engine-setup-note")!;
// Whether the model each local engine is set to use is actually on disk. Cached from the
// last backend check so the setup note can re-render without re-querying.
let whisperModelPresent = false;
let parakeetModelPresent = false;
const cloudModelSettings = document.getElementById("cloud-model-settings")!;
const modelFast = document.getElementById("model-fast")!;
const modelAccurate = document.getElementById("model-accurate")!;
const aiOff = document.getElementById("ai-off")!;
const aiOn = document.getElementById("ai-on")!;
const bgOff = document.getElementById("bg-off")!;
const bgOn = document.getElementById("bg-on")!;
const autostartOff = document.getElementById("autostart-off")!;
const autostartOn = document.getElementById("autostart-on")!;
const aiModelSelect = document.getElementById("ai-model-select") as HTMLSelectElement;
const aiProfileCleanup = document.getElementById("ai-profile-cleanup")!;
const aiProfilePrompt = document.getElementById("ai-profile-prompt")!;
const aiProfileAuto = document.getElementById("ai-profile-auto")!;
const aiFormatSettings = document.getElementById("ai-format-settings")!;
const autoContextRow = document.getElementById("auto-context-row")!;
const aiOptions = document.getElementById("ai-options")!;
const aiDisabledNote = document.getElementById("ai-disabled-note")!;
const aiToneDefault = document.getElementById("ai-tone-default")!;
const aiToneFormal = document.getElementById("ai-tone-formal")!;
const aiToneCasual = document.getElementById("ai-tone-casual")!;
const aiToneConcise = document.getElementById("ai-tone-concise")!;
const aiStyleDefault = document.getElementById("ai-style-default")!;
const aiStyleBullets = document.getElementById("ai-style-bullets")!;
const aiStyleParagraphs = document.getElementById("ai-style-paragraphs")!;
const aiStyleRaw = document.getElementById("ai-style-raw")!;
const aiCustomInstructions = document.getElementById("ai-custom-instructions") as HTMLTextAreaElement;
const autoContextOverride = document.getElementById("auto-context-override") as HTMLSelectElement;
const appRuleProcess = document.getElementById("app-rule-process") as HTMLInputElement;
const appPickerPanel = document.getElementById("app-picker-panel") as HTMLDivElement;
const appRuleTitle = document.getElementById("app-rule-title") as HTMLInputElement;
const appRuleCategory = document.getElementById("app-rule-category") as HTMLSelectElement;
const appRuleAddBtn = document.getElementById("app-rule-add-btn")!;
const appRulesList = document.getElementById("app-rules-list")!;
const presetEmail = document.getElementById("preset-email")!;
const presetChat = document.getElementById("preset-chat")!;
const presetNotes = document.getElementById("preset-notes")!;
const presetBrief = document.getElementById("preset-brief")!;
const presetReset = document.getElementById("preset-reset")!;
const aiFormatNatural = document.getElementById("ai-format-natural")!;
const aiFormatStructured = document.getElementById("ai-format-structured")!;
const modeToggle = document.getElementById("mode-toggle")!;
const modePtt = document.getElementById("mode-ptt")!;
const hotkeyDisplay = document.getElementById("hotkey-display") as HTMLElement;
const hotkeyChangeBtn = document.getElementById("hotkey-change-btn") as HTMLButtonElement;
const hotkeyResetBtn = document.getElementById("hotkey-reset-btn") as HTMLButtonElement;
// Transient status line only. The persistent instructions live in #hotkey-hint,
// which is static markup and never touched here, so they are always visible.
const hotkeyStatus = document.getElementById("hotkey-status") as HTMLElement;
const hotkey2Display = document.getElementById("hotkey2-display") as HTMLElement;
const hotkey2ChangeBtn = document.getElementById("hotkey2-change-btn") as HTMLButtonElement;
const hotkey2ClearBtn = document.getElementById("hotkey2-clear-btn") as HTMLButtonElement;
const hotkey2Status = document.getElementById("hotkey2-status") as HTMLElement;
const secondaryProfileCleanup = document.getElementById("secondary-profile-cleanup") as HTMLButtonElement;
const secondaryProfilePrompt = document.getElementById("secondary-profile-prompt") as HTMLButtonElement;
const secondaryProfileAuto = document.getElementById("secondary-profile-auto") as HTMLButtonElement;
const statCount = document.getElementById("stat-count")!;
const statWords = document.getElementById("stat-words")!;
const statWpm = document.getElementById("stat-wpm")!;
const transcriptionFeed = document.getElementById("transcription-feed")!;
const historySearch = document.getElementById("history-search") as HTMLInputElement;
let historyQuery = "";
historySearch.addEventListener("input", () => {
  historyQuery = historySearch.value;
  visibleHistoryCount = 50; // reset pagination when the query changes
  loadHistory(false);
});

// The history the user is currently looking at: filtered if a search is
// active, otherwise everything.
function currentHistoryView(): TranscriptionItem[] {
  const all = cachedHistory?.items ?? [];
  const q = historyQuery.trim().toLowerCase();
  return q ? all.filter(it => it.text.toLowerCase().includes(q)) : all;
}

function buildHistoryJson(items: TranscriptionItem[]): string {
  return JSON.stringify(items, null, 2);
}

function buildHistoryMarkdown(items: TranscriptionItem[]): string {
  let out = "# Typr History Export\n";
  let lastGroup = "";
  items.forEach(item => {
    const date = new Date(item.timestamp * 1000);
    const groupKey = date.toLocaleDateString(undefined, { weekday: "long", month: "short", day: "numeric", year: "numeric" });
    if (groupKey !== lastGroup) {
      out += `\n## ${groupKey}\n\n`;
      lastGroup = groupKey;
    }
    const timeStr = date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    out += `**${timeStr}** — ${item.text}\n\n`;
  });
  return out;
}

// Simple auto-dismissing toast (reuses the .undo-toast style, no button).
function showToast(message: string, ms = 3000) {
  const toast = document.createElement("div");
  toast.className = "undo-toast";
  toast.textContent = message;
  document.body.appendChild(toast);
  setTimeout(() => toast.remove(), ms);
}

async function exportHistory(format: "json" | "markdown") {
  const items = currentHistoryView();
  if (items.length === 0) {
    showToast("Nothing to export");
    return;
  }
  const ext = format === "json" ? "json" : "md";
  const today = new Date().toISOString().slice(0, 10);
  const path = await save({
    defaultPath: `typr-history-${today}.${ext}`,
    filters: [{ name: format === "json" ? "JSON" : "Markdown", extensions: [ext] }],
  });
  if (!path) return; // user cancelled
  const contents = format === "json" ? buildHistoryJson(items) : buildHistoryMarkdown(items);
  try {
    await invoke("write_text_file", { path, contents });
    showToast(`Exported ${items.length} transcription${items.length === 1 ? "" : "s"}`);
  } catch (err) {
    console.error(err);
    showToast(`Export failed: ${String(err)}`);
  }
}

document.getElementById("export-json-btn")!.addEventListener("click", () => exportHistory("json"));
document.getElementById("export-md-btn")!.addEventListener("click", () => exportHistory("markdown"));

// Section navigation
const navItems = document.querySelectorAll(".nav-item");
const sections = document.querySelectorAll(".content-section");

navItems.forEach((item) => {
  item.addEventListener("click", () => {
    const target = item.getAttribute("data-section");
    // If a hotkey capture is in progress, cancel it so the suspended global
    // shortcuts get re-armed instead of being left dead when we navigate away.
    if (primaryCapture.isCapturing()) primaryCapture.cancel();
    if (secondaryCapture.isCapturing()) secondaryCapture.cancel();
    // Clear any lingering transient status so the tab reads fresh on entry
    // (the persistent instructions in #hotkey-hint always remain visible).
    primaryCapture.clearStatus();
    secondaryCapture.clearStatus();
    navItems.forEach((n) => n.classList.remove("active"));
    sections.forEach((s) => s.classList.remove("active"));
    item.classList.add("active");
    document.getElementById(`section-${target}`)?.classList.add("active");
  });
});

// Window drag & controls — titlebar and sidebar empty space
const titlebar = document.getElementById("titlebar")!;
const sidebar = document.getElementById("sidebar")!;
const appWindow = getCurrentWindow();

titlebar.addEventListener("mousedown", (e) => {
  if ((e.target as HTMLElement).closest("button, select, input, a, .nav-item, .titlebar-button")) return;
  appWindow.startDragging();
});

sidebar.addEventListener("mousedown", (e) => {
  if ((e.target as HTMLElement).closest("button, select, input, a, .nav-item")) return;
  appWindow.startDragging();
});

// Sidebar collapse. Pure window chrome, so it lives in localStorage rather than
// the backend Settings — nothing here needs to survive a reinstall.
const collapseBtn = document.getElementById("collapse-btn")!;
const COLLAPSE_KEY = "sidebarCollapsed";

function applyCollapsed(collapsed: boolean) {
  sidebar.classList.toggle("collapsed", collapsed);
  const label = collapsed ? "Expand sidebar" : "Collapse sidebar";
  collapseBtn.setAttribute("title", label);
  collapseBtn.setAttribute("aria-label", label);
  collapseBtn.setAttribute("aria-expanded", String(!collapsed));
}

collapseBtn.addEventListener("click", () => {
  const collapsed = !sidebar.classList.contains("collapsed");
  localStorage.setItem(COLLAPSE_KEY, String(collapsed));
  applyCollapsed(collapsed);
});

// Restore without animating: otherwise a collapsed sidebar visibly slides shut
// on every launch.
sidebar.classList.add("no-anim");
applyCollapsed(localStorage.getItem(COLLAPSE_KEY) === "true");
requestAnimationFrame(() => sidebar.classList.remove("no-anim"));

// Custom window controls handlers
const minimizeBtn = document.getElementById("titlebar-minimize")!;
const maximizeBtn = document.getElementById("titlebar-maximize")!;
const closeBtn = document.getElementById("titlebar-close")!;
const maximizeIcon = document.getElementById("maximize-icon")!;
const restoreIcon = document.getElementById("restore-icon")!;

minimizeBtn.addEventListener("click", () => {
  appWindow.minimize();
});

maximizeBtn.addEventListener("click", () => {
  appWindow.toggleMaximize();
});

closeBtn.addEventListener("click", () => {
  appWindow.close();
});

async function updateMaximizeIcon() {
  const isMax = await appWindow.isMaximized();
  if (isMax) {
    maximizeIcon.classList.add("hidden");
    restoreIcon.classList.remove("hidden");
  } else {
    maximizeIcon.classList.remove("hidden");
    restoreIcon.classList.add("hidden");
  }
}

// Check initial window state
updateMaximizeIcon();

// Listen for resize to update state
appWindow.onResized(() => {
  updateMaximizeIcon();
});

let currentSettings: Settings;

async function loadSettings() {
  currentSettings = await invoke<Settings>("get_settings");
  await loadHistory();
  await loadDictionary();

  // Populate mic dropdown
  await populateMics();

  // Engine
  setEngine(currentSettings.engine);

  // Model — select the stored model. All four options (full + English-quantized, small +
  // medium) are valid selections, so an existing "small"/"medium" choice is kept as-is.
  // Only rescue a truly unknown/blank value so the dropdown never shows empty.
  modelSelect.value = currentSettings.whisperModel;
  if (modelSelect.selectedIndex === -1) {
    modelSelect.value = "medium.en-q5_0";
    // Persist via the full loaded object so we don't clobber the groq key / cloud model
    // that haven't been written to the DOM yet at this point in loadSettings.
    currentSettings.whisperModel = modelSelect.value;
    await invoke("save_settings", { settings: currentSettings });
  }
  await checkModelStatus();

  // Groq key — mirrored into both the Engine and AI tab inputs.
  syncGroqKeyInputs(currentSettings.groqApiKey);

  // Cloud model
  setCloudModel(currentSettings.cloudModel || "accurate");
  setParakeetModel(currentSettings.parakeetModel || "v3");
  refreshParakeetDownloadState();

  // Recording mode
  setRecordingMode(currentSettings.recordingMode);

  // Hotkey
  hotkeyDisplay.textContent = currentSettings.hotkey.replace("CmdOrCtrl", "Cmd");
  renderSecondaryHotkey();
  renderSecondaryProfile();

  // Startup & background
  setBackgroundMode(currentSettings.backgroundMode);
  setAutostart(currentSettings.autostart);

  // AI post-processing
  setAiEnabled(currentSettings.aiEnabled);
  setAiModel(currentSettings.aiModel || AI_MODEL_DEFAULT);
  setAiProfile(currentSettings.aiProfile || "cleanup");
  setAiPromptFormat(currentSettings.aiPromptFormat || "natural");
  setAiTone(currentSettings.aiTone || "default");
  setAiFormat(currentSettings.aiFormat || "default");
  aiCustomInstructions.value = currentSettings.aiCustomInstructions || "";

  // Auto context override + app rules
  autoContextOverride.value = currentSettings.autoContextOverride || "auto";
  await loadAppRules();
}

async function populateMics() {
  const mics = await invoke<MicDevice[]>("list_microphones");
  const previous = currentSettings.microphone;
  micSelect.innerHTML = "";
  mics.forEach((mic) => {
    const option = document.createElement("option");
    option.value = mic.name;
    option.textContent = mic.name + (mic.is_default ? " (default)" : "");
    micSelect.appendChild(option);
  });
  micSelect.value = previous;
}

// Mirrors the dispatch arms in recorder.rs. An unrecognized value falls back to local so a
// stale config can never leave the engine unset.
function setEngine(engine: string) {
  const e = engine === "cloud" ? "cloud" : engine === "parakeet" ? "parakeet" : "local";
  currentSettings.engine = e;
  engineLocal.classList.toggle("active", e === "local");
  engineParakeet.classList.toggle("active", e === "parakeet");
  engineCloud.classList.toggle("active", e === "cloud");
  localSettings.classList.toggle("hidden", e !== "local");
  parakeetSettings.classList.toggle("hidden", e !== "parakeet");
  cloudSettings.classList.toggle("hidden", e !== "cloud");
  cloudModelSettings.classList.toggle("hidden", e !== "cloud");
  // A progress bar shows only on its own engine's tab, and only while that engine is
  // actually downloading — so switching tabs mid-download never strands a bar in the
  // wrong place, and returning to the tab shows it again at its real progress.
  downloadProgress.classList.toggle("hidden", !(e === "local" && whisperDownloading));
  parakeetDownloadProgress.classList.toggle("hidden", !(e === "parakeet" && parakeetDownloading));
  updateEngineSetupNote();
}

function setParakeetModel(variant: string) {
  const v = variant === "v2" ? "v2" : "v3";
  currentSettings.parakeetModel = v;
  parakeetModelSelect.value = v;
}

/// Reflect whether the selected variant is on disk, so the button says what it will do.
async function refreshParakeetDownloadState() {
  try {
    const present = await invoke<boolean>("check_parakeet_downloaded", {
      variant: currentSettings.parakeetModel || "v3",
    });
    parakeetDownloadBtn.textContent = present ? "Re-download" : "Download";
    parakeetModelPresent = present;
  } catch {
    parakeetDownloadBtn.textContent = "Download";
  }
  updateEngineSetupNote();
}

/// First-run guidance: name the one thing still missing for the selected engine. A new user
/// otherwise sees an ordinary settings page with nothing to indicate that local transcription
/// needs a model downloaded before the hotkey will produce anything at all.
function updateEngineSetupNote() {
  let msg = "";
  if (currentSettings.engine === "local" && !whisperModelPresent) {
    msg =
      "No Whisper model downloaded yet — dictation won't work until you download one. Pick a size above and click Download.";
  } else if (currentSettings.engine === "parakeet" && !parakeetModelPresent) {
    msg =
      "No Parakeet model downloaded yet — dictation won't work until you download one. Click Download above (about 640 MB).";
  } else if (currentSettings.engine === "cloud" && !(currentSettings.groqApiKey || "").trim()) {
    msg =
      "No Groq API key set — cloud transcription won't work until you add one. Get a free key at console.groq.com (Groq, not grok.com).";
  }
  engineSetupNote.textContent = msg;
  engineSetupNote.classList.toggle("hidden", !msg);
}

function setCloudModel(model: string) {
  const m = model === "fast" ? "fast" : "accurate";
  currentSettings.cloudModel = m;
  modelFast.classList.toggle("active", m === "fast");
  modelAccurate.classList.toggle("active", m === "accurate");
}

function setAiEnabled(enabled: boolean) {
  currentSettings.aiEnabled = enabled;
  aiOff.classList.toggle("active", !enabled);
  aiOn.classList.toggle("active", enabled);
  // Every setting below the toggle only takes effect when AI is on. Leaving them
  // visible while off invites the reader to change them and wonder why nothing
  // happens, so hide the lot and say why in their place.
  aiOptions.classList.toggle("hidden", !enabled);
  aiDisabledNote.classList.toggle("hidden", enabled);
  updateAiKeyWarning();
}

/// Keep the Engine-tab and AI-tab key inputs showing one value, and refresh the warning.
function syncGroqKeyInputs(value: string) {
  value = value || "";
  currentSettings.groqApiKey = value;
  if (groqKey.value !== value) groqKey.value = value;
  if (aiGroqKey.value !== value) aiGroqKey.value = value;
  updateAiKeyWarning();
  updateEngineSetupNote();
}

/// Say so when AI cleanup is on but has no key. Without this the failure is invisible:
/// the pipeline treats a missing key as a failed AI call, falls back to deterministic
/// cleanup, and records the reason only in the debug log — so the feature looks broken.
function updateAiKeyWarning() {
  // Tolerate a config written by an older build that has no key field at all: a throw here
  // would abort loadSettings and leave the whole window half-populated.
  const missing = currentSettings.aiEnabled && !(currentSettings.groqApiKey || "").trim();
  aiKeyMissingNote.classList.toggle("hidden", !missing);
}

function setBackgroundMode(enabled: boolean) {
  currentSettings.backgroundMode = enabled;
  bgOff.classList.toggle("active", !enabled);
  bgOn.classList.toggle("active", enabled);
}

function setAutostart(enabled: boolean) {
  currentSettings.autostart = enabled;
  autostartOff.classList.toggle("active", !enabled);
  autostartOn.classList.toggle("active", enabled);
  // Auto-start implies background — reflect the forced state in the UI.
  if (enabled) setBackgroundMode(true);
}

// Mirrors resolve_model() in ai_postprocess.rs — keep the two allowlists in step, or the
// dropdown silently coerces a valid choice back to the default.
const AI_MODELS = ["qwen/qwen3.6-27b", "openai/gpt-oss-20b", "openai/gpt-oss-120b"];
const AI_MODEL_DEFAULT = "qwen/qwen3.6-27b";

function setAiModel(model: string) {
  const m = AI_MODELS.includes(model) ? model : AI_MODEL_DEFAULT;
  currentSettings.aiModel = m;
  aiModelSelect.value = m;
}

function setAiProfile(profile: string) {
  const p = profile === "prompt" ? "prompt" : profile === "auto" ? "auto" : "cleanup";
  currentSettings.aiProfile = p;
  aiProfileCleanup.classList.toggle("active", p === "cleanup");
  aiProfilePrompt.classList.toggle("active", p === "prompt");
  aiProfileAuto.classList.toggle("active", p === "auto");
  // Both sub-selectors qualify the profile above them, so each is shown only for
  // the profile it belongs to. The context picker was previously never toggled at
  // all and sat visible under Cleanup and Prompt Mode, where it does nothing.
  aiFormatSettings.classList.toggle("hidden", p !== "prompt");
  autoContextRow.classList.toggle("hidden", p !== "auto");
}

function setAiPromptFormat(format: string) {
  const f = format === "structured" ? "structured" : "natural";
  currentSettings.aiPromptFormat = f;
  aiFormatNatural.classList.toggle("active", f === "natural");
  aiFormatStructured.classList.toggle("active", f === "structured");
}

function setAiTone(tone: string) {
  const t = ["formal", "casual", "concise"].includes(tone) ? tone : "default";
  currentSettings.aiTone = t;
  aiToneDefault.classList.toggle("active", t === "default");
  aiToneFormal.classList.toggle("active", t === "formal");
  aiToneCasual.classList.toggle("active", t === "casual");
  aiToneConcise.classList.toggle("active", t === "concise");
}

function setAiFormat(format: string) {
  const f = ["bullets", "paragraphs", "raw"].includes(format) ? format : "default";
  currentSettings.aiFormat = f;
  aiStyleDefault.classList.toggle("active", f === "default");
  aiStyleBullets.classList.toggle("active", f === "bullets");
  aiStyleParagraphs.classList.toggle("active", f === "paragraphs");
  aiStyleRaw.classList.toggle("active", f === "raw");
}

// One-click presets set the Tone + Formatting toggles together (no separate backend).
function applyPreset(tone: string, format: string) {
  setAiTone(tone);
  setAiFormat(format);
  saveSettings();
}

function setRecordingMode(mode: string) {
  currentSettings.recordingMode = mode;
  modeToggle.classList.toggle("active", mode === "toggle");
  modePtt.classList.toggle("active", mode === "push-to-talk");
}

async function checkModelStatus() {
  const downloaded = await invoke<boolean>("check_model_downloaded", {
    modelSize: modelSelect.value,
  });
  // Standardized with Parakeet: a present model offers "Re-download" (still clickable) rather
  // than a disabled checkmark, so a corrupted or interrupted file can be re-fetched from the UI.
  downloadBtn.textContent = downloaded ? "Re-download" : "Download";
  (downloadBtn as HTMLButtonElement).disabled = false;
  whisperModelPresent = downloaded;
  updateEngineSetupNote();
}

async function saveSettings() {
  currentSettings.microphone = micSelect.value;
  currentSettings.whisperModel = modelSelect.value;
  // Both key inputs are mirrored, so either is authoritative; syncing here keeps a save
  // triggered by some other control from writing back a stale value.
  syncGroqKeyInputs(currentSettings.groqApiKey);
  currentSettings.aiCustomInstructions = aiCustomInstructions.value;
  await invoke("save_settings", { settings: currentSettings });
}

// Event listeners
engineLocal.addEventListener("click", () => {
  setEngine("local");
  saveSettings();
});

engineCloud.addEventListener("click", () => {
  setEngine("cloud");
  saveSettings();
});

engineParakeet.addEventListener("click", () => {
  setEngine("parakeet");
  saveSettings();
  refreshParakeetDownloadState();
});

parakeetModelSelect.addEventListener("change", () => {
  setParakeetModel(parakeetModelSelect.value);
  saveSettings();
  refreshParakeetDownloadState();
});

parakeetDownloadBtn.addEventListener("click", async () => {
  const variant = currentSettings.parakeetModel || "v3";
  parakeetDownloadBtn.disabled = true;
  parakeetDownloadBtn.textContent = "Downloading…";
  parakeetDownloading = true;
  activeProgressFill = parakeetProgressFill;
  parakeetProgressFill.style.width = "0%";
  parakeetDownloadProgress.classList.remove("hidden");
  try {
    await invoke("download_parakeet_model", { variant });
    showToast("Parakeet model ready");
  } catch (e) {
    showToast(String(e));
  } finally {
    parakeetDownloading = false;
    if (activeProgressFill === parakeetProgressFill) activeProgressFill = null;
    parakeetDownloadProgress.classList.add("hidden");
    parakeetDownloadBtn.disabled = false;
    // refreshParakeetDownloadState sets the label to "Re-download" (present) or "Download".
    refreshParakeetDownloadState();
  }
});

micSelect.addEventListener("change", () => saveSettings());

micSelect.addEventListener("mousedown", () => {
  populateMics();
});

modelSelect.addEventListener("change", async () => {
  await checkModelStatus();
  saveSettings();
});

downloadBtn.addEventListener("click", async () => {
  (downloadBtn as HTMLButtonElement).disabled = true;
  downloadBtn.textContent = "Downloading\u2026";
  whisperDownloading = true;
  activeProgressFill = progressFill;
  progressFill.style.width = "0%";
  downloadProgress.classList.remove("hidden");

  try {
    await invoke("download_model", { modelSize: modelSelect.value });
    downloadBtn.textContent = "Re-download";
    // The model is on disk now, so clear the first-run note. Set directly rather than via
    // checkModelStatus() so the "Retry" label on the failure path below is left alone.
    whisperModelPresent = true;
    updateEngineSetupNote();
  } catch (e) {
    downloadBtn.textContent = "Retry";
    console.error("Download failed:", e);
  }
  whisperDownloading = false;
  if (activeProgressFill === progressFill) activeProgressFill = null;
  (downloadBtn as HTMLButtonElement).disabled = false;
  downloadProgress.classList.add("hidden");
});

// Mirror on every keystroke so the warning clears as soon as a key is pasted, and persist
// on change (blur) as before — one setting, edited from either tab.
groqKey.addEventListener("input", () => syncGroqKeyInputs(groqKey.value));
aiGroqKey.addEventListener("input", () => syncGroqKeyInputs(aiGroqKey.value));
groqKey.addEventListener("change", () => saveSettings());
aiGroqKey.addEventListener("change", () => saveSettings());

aiOff.addEventListener("click", () => {
  setAiEnabled(false);
  saveSettings();
});

aiOn.addEventListener("click", () => {
  setAiEnabled(true);
  saveSettings();
});

bgOff.addEventListener("click", () => {
  setBackgroundMode(false);
  saveSettings();
});
bgOn.addEventListener("click", () => {
  setBackgroundMode(true);
  saveSettings();
});
autostartOff.addEventListener("click", () => {
  setAutostart(false);
  saveSettings();
});
autostartOn.addEventListener("click", () => {
  setAutostart(true);
  saveSettings();
});

aiModelSelect.addEventListener("change", () => {
  setAiModel(aiModelSelect.value);
  saveSettings();
});

aiProfileCleanup.addEventListener("click", () => {
  setAiProfile("cleanup");
  saveSettings();
});

aiProfilePrompt.addEventListener("click", () => {
  setAiProfile("prompt");
  saveSettings();
});

aiToneDefault.addEventListener("click", () => { setAiTone("default"); saveSettings(); });
aiToneFormal.addEventListener("click", () => { setAiTone("formal"); saveSettings(); });
aiToneCasual.addEventListener("click", () => { setAiTone("casual"); saveSettings(); });
aiToneConcise.addEventListener("click", () => { setAiTone("concise"); saveSettings(); });
aiStyleDefault.addEventListener("click", () => { setAiFormat("default"); saveSettings(); });
aiStyleBullets.addEventListener("click", () => { setAiFormat("bullets"); saveSettings(); });
aiStyleParagraphs.addEventListener("click", () => { setAiFormat("paragraphs"); saveSettings(); });
aiStyleRaw.addEventListener("click", () => { setAiFormat("raw"); saveSettings(); });
aiCustomInstructions.addEventListener("change", () => saveSettings());

autoContextOverride.addEventListener("change", () => {
  currentSettings.autoContextOverride = autoContextOverride.value;
  saveSettings();
});

presetEmail.addEventListener("click", () => applyPreset("formal", "paragraphs"));
presetChat.addEventListener("click", () => applyPreset("casual", "default"));
presetNotes.addEventListener("click", () => applyPreset("default", "bullets"));
presetBrief.addEventListener("click", () => applyPreset("concise", "default"));
presetReset.addEventListener("click", () => applyPreset("default", "default"));

aiProfileAuto.addEventListener("click", () => {
  setAiProfile("auto");
  saveSettings();
});

aiFormatNatural.addEventListener("click", () => {
  setAiPromptFormat("natural");
  saveSettings();
});

aiFormatStructured.addEventListener("click", () => {
  setAiPromptFormat("structured");
  saveSettings();
});

modelFast.addEventListener("click", () => {
  setCloudModel("fast");
  saveSettings();
});

modelAccurate.addEventListener("click", () => {
  setCloudModel("accurate");
  saveSettings();
});

modeToggle.addEventListener("click", () => {
  setRecordingMode("toggle");
  saveSettings();
});

modePtt.addEventListener("click", () => {
  setRecordingMode("push-to-talk");
  saveSettings();
});

// --- Hotkey capture ---------------------------------------------------------
const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "Meta", "OS"]);

/// Modifiers currently held, in Tauri accelerator order. `e.metaKey` maps to
/// Super (the Windows key), NOT CmdOrCtrl.
///
/// On Windows, Ctrl+Alt is delivered as AltGr: for keys that have an AltGr
/// character on the active layout (e.g. H→ḥ, Q→æ) the letter keydown reports
/// `ctrlKey=false, altKey=false` but `AltGraph=true`. Treat AltGr as its
/// physical components — Ctrl+Alt — so those combos capture like any other.
function modifiersFromEvent(e: KeyboardEvent): string[] {
  const altGraph = e.getModifierState("AltGraph");
  const mods: string[] = [];
  if (e.ctrlKey || altGraph) mods.push("CmdOrCtrl");
  if (e.altKey || altGraph) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  return mods;
}

/// Map a physical `KeyboardEvent.code` to a Tauri accelerator key token,
/// layout- and Shift-independent. Returns `null` for keys we don't support as
/// hotkey targets (so we can reject them with clear guidance instead of
/// emitting an accelerator the backend can't register).
function codeToKeyToken(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3); // KeyD -> D
  if (/^Digit[0-9]$/.test(code)) return code.slice(5); // Digit1 -> 1
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code; // F1..F24
  switch (code) {
    case "Space":
      return "Space";
    case "ArrowUp":
      return "Up";
    case "ArrowDown":
      return "Down";
    case "ArrowLeft":
      return "Left";
    case "ArrowRight":
      return "Right";
    case "Home":
      return "Home";
    case "End":
      return "End";
    case "PageUp":
      return "PageUp";
    case "PageDown":
      return "PageDown";
    case "Insert":
      return "Insert";
    case "Delete":
      return "Delete";
    default:
      return null;
  }
}

function toDisplay(accel: string): string {
  return accel.replace("CmdOrCtrl", "Cmd").replace("Super", "Win").split("+").join(" + ");
}

/// A capture control bound to one hotkey (primary or secondary). Reuses the
/// shared normalization/AltGr/suspend logic; only the DOM elements, the bind
/// command, and the current-value accessor differ per instance.
interface HotkeyCaptureConfig {
  display: HTMLElement;
  status: HTMLElement;
  changeBtn: HTMLButtonElement;
  setCommand: string; // "set_hotkey" | "set_secondary_hotkey"
  getCurrent: () => string;
  onSet: (accepted: string) => void;
  onCurrentRender?: () => void; // optional override for how the idle value renders
}

function createHotkeyCapture(config: HotkeyCaptureConfig) {
  let capturing = false;

  const renderCurrent = () => {
    if (config.onCurrentRender) config.onCurrentRender();
    else config.display.textContent = config.getCurrent().replace("CmdOrCtrl", "Cmd");
  };
  const setStatus = (msg: string) => {
    config.status.textContent = msg;
  };
  const clearStatus = () => {
    config.status.textContent = "";
  };
  const renderPreview = (mods: string[], key: string | null) => {
    const parts = key ? [...mods, key] : [...mods, "…"];
    config.display.textContent = toDisplay(parts.join("+"));
  };

  async function onKeydown(e: KeyboardEvent): Promise<void> {
    if (!capturing) return;
    e.preventDefault();
    e.stopPropagation();

    if (e.key === "Escape") {
      await cancel();
      return;
    }

    const mods = modifiersFromEvent(e);
    if (MODIFIER_KEYS.has(e.key)) {
      renderPreview(mods, null);
      setStatus("Listening… now press a key (Esc to cancel).");
      return;
    }

    const key = codeToKeyToken(e.code);
    if (key === null) {
      setStatus("That key can't be used — try a letter, number, F-key, arrow, or Space.");
      return;
    }
    if (mods.length === 0) {
      renderPreview([], key);
      setStatus("Add a modifier — Ctrl, Alt, or Shift — then this key.");
      return;
    }

    const accel = [...mods, key].join("+");
    renderPreview(mods, key);
    await commit(accel);
  }

  async function commit(accel: string): Promise<void> {
    endListening();
    try {
      const accepted = await invoke<string>(config.setCommand, { accelerator: accel });
      config.onSet(accepted);
      renderCurrent();
      setStatus(`Hotkey set to ${toDisplay(accepted)}.`);
    } catch (err) {
      console.error(`[Typr] ${config.setCommand} failed:`, err);
      renderCurrent();
      setStatus(
        `${toDisplay(accel)} is already in use by Windows or another app — try another. Your hotkey is unchanged.`,
      );
    }
  }

  async function cancel(): Promise<void> {
    endListening();
    try {
      await invoke("resume_hotkeys"); // re-arm the hotkeys we suspended for capture
    } catch (err) {
      console.error("[Typr] resume_hotkeys failed:", err);
    }
    renderCurrent();
    clearStatus();
  }

  function endListening(): void {
    capturing = false;
    config.changeBtn.textContent = "Change";
    window.removeEventListener("keydown", onKeydown, true);
  }

  async function start(): Promise<void> {
    config.changeBtn.textContent = "Listening…";
    setStatus("Listening… hold your modifiers and press a key (Esc to cancel).");
    renderPreview([], null);
    try {
      await invoke("suspend_hotkeys"); // stop the active hotkeys from eating keys
    } catch (err) {
      console.error("[Typr] suspend_hotkeys failed:", err);
    }
    capturing = true;
    window.addEventListener("keydown", onKeydown, true);
  }

  return {
    start: () => void start(),
    cancel: () => void cancel(),
    isCapturing: () => capturing,
    clearStatus,
  };
}

const primaryCapture = createHotkeyCapture({
  display: hotkeyDisplay,
  status: hotkeyStatus,
  changeBtn: hotkeyChangeBtn,
  setCommand: "set_hotkey",
  getCurrent: () => currentSettings.hotkey,
  onSet: (accepted) => {
    currentSettings.hotkey = accepted;
  },
});

hotkeyChangeBtn.addEventListener("click", () => {
  if (primaryCapture.isCapturing()) primaryCapture.cancel();
  else primaryCapture.start();
});

hotkeyResetBtn.addEventListener("click", async () => {
  if (primaryCapture.isCapturing()) primaryCapture.cancel();
  try {
    const accepted = await invoke<string>("set_hotkey", {
      accelerator: "CmdOrCtrl+Shift+Space",
    });
    currentSettings.hotkey = accepted;
    hotkeyDisplay.textContent = accepted.replace("CmdOrCtrl", "Cmd");
    hotkeyStatus.textContent = "Hotkey reset to Cmd+Shift+Space.";
  } catch (err) {
    hotkeyStatus.textContent = String(err);
  }
});

// --- Secondary (AI) hotkey --------------------------------------------------
function renderSecondaryHotkey(): void {
  const v = currentSettings.hotkeySecondary;
  hotkey2Display.textContent = v ? v.replace("CmdOrCtrl", "Cmd") : "Not set";
  hotkey2ClearBtn.disabled = !v;
}

function renderSecondaryProfile(): void {
  const p = currentSettings.secondaryProfile || "prompt";
  secondaryProfileCleanup.classList.toggle("active", p === "cleanup");
  secondaryProfilePrompt.classList.toggle("active", p === "prompt");
  secondaryProfileAuto.classList.toggle("active", p === "auto");
}

const secondaryCapture = createHotkeyCapture({
  display: hotkey2Display,
  status: hotkey2Status,
  changeBtn: hotkey2ChangeBtn,
  setCommand: "set_secondary_hotkey",
  getCurrent: () => currentSettings.hotkeySecondary || "",
  onSet: (accepted) => {
    currentSettings.hotkeySecondary = accepted;
    renderSecondaryHotkey();
  },
  // Unset renders "Not set" rather than an empty box (used on cancel/idle).
  onCurrentRender: renderSecondaryHotkey,
});

hotkey2ChangeBtn.addEventListener("click", () => {
  if (secondaryCapture.isCapturing()) secondaryCapture.cancel();
  else secondaryCapture.start();
});

hotkey2ClearBtn.addEventListener("click", async () => {
  if (secondaryCapture.isCapturing()) secondaryCapture.cancel();
  try {
    await invoke("clear_secondary_hotkey");
    currentSettings.hotkeySecondary = "";
    renderSecondaryHotkey();
    hotkey2Status.textContent = "AI hotkey cleared.";
  } catch (err) {
    hotkey2Status.textContent = String(err);
  }
});

function wireSecondaryProfile(btn: HTMLButtonElement, profile: string): void {
  btn.addEventListener("click", async () => {
    try {
      await invoke("set_secondary_profile", { profile });
      currentSettings.secondaryProfile = profile;
      renderSecondaryProfile();
    } catch (err) {
      hotkey2Status.textContent = String(err);
    }
  });
}
wireSecondaryProfile(secondaryProfileCleanup, "cleanup");
wireSecondaryProfile(secondaryProfilePrompt, "prompt");
wireSecondaryProfile(secondaryProfileAuto, "auto");

// Label reflecting the true recording state, so the transient mic notice can
// restore to it instead of blindly resetting to "Ready" mid-recording.
let recordingLabel = "Ready";

// Brief notice when the active input device changes or falls back
let micNoticeTimer: number | undefined;
listen<{ device: string; fellBack: boolean }>("mic-changed", (event) => {
  const { device, fellBack } = event.payload;
  statusText.textContent = fellBack ? `Switched to ${device}` : `Using ${device}`;
  if (micNoticeTimer !== undefined) clearTimeout(micNoticeTimer);
  micNoticeTimer = window.setTimeout(() => {
    statusText.textContent = recordingLabel;
  }, 2000);
});

// Listen for recording state changes
listen<string>("recording-state", (event) => {
  const state = event.payload;
  statusDot.className = "";
  statusIndicator.className = "";
  // A real state transition takes authority over any pending mic notice.
  if (micNoticeTimer !== undefined) {
    clearTimeout(micNoticeTimer);
    micNoticeTimer = undefined;
  }
  if (state === "Recording") {
    statusDot.classList.add("recording");
    statusIndicator.classList.add("recording");
    recordingLabel = "Recording...";
  } else if (state === "Transcribing") {
    statusDot.classList.add("transcribing");
    statusIndicator.classList.add("transcribing");
    recordingLabel = "Transcribing...";
  } else {
    statusDot.classList.add("ready");
    statusIndicator.classList.add("ready");
    recordingLabel = "Ready";
  }
  statusText.textContent = recordingLabel;
});

// Listen for download progress. Routes to whichever engine's bar is currently downloading,
// so the Whisper and Parakeet fills never cross-update.
listen<DownloadProgress>("download-progress", (event) => {
  const { percent } = event.payload;
  if (activeProgressFill) activeProgressFill.style.width = `${percent}%`;
});

// Listen for history updates
listen("history-updated", () => {
  loadHistory();
});

let visibleHistoryCount = 50;
let cachedHistory: History | null = null;

interface Correction { find: string; replace: string; }

// Transient toast with a 5s Undo window. onExpire fires if the user does not
// click Undo before the window closes (used to commit a delete to disk).
function showUndoToast(
  message: string,
  onUndo: () => void,
  onExpire: () => void,
  ms = 5000,
) {
  const toast = document.createElement("div");
  toast.className = "undo-toast";

  const msg = document.createElement("span");
  msg.textContent = message;

  const undoBtn = document.createElement("button");
  undoBtn.className = "undo-toast-btn";
  undoBtn.textContent = "Undo";

  toast.appendChild(msg);
  toast.appendChild(undoBtn);
  document.body.appendChild(toast);

  let done = false;
  const timer = window.setTimeout(() => {
    if (done) return;
    done = true;
    toast.remove();
    onExpire();
  }, ms);

  undoBtn.onclick = () => {
    if (done) return;
    done = true;
    clearTimeout(timer);
    toast.remove();
    onUndo();
  };
}

// After a save that changed the text, ask the backend for the corrected term
// and, if there is one, offer to add it to the Dictionary as a spelling hint.
// A hint only *biases* the engine toward that spelling — it never rewrites text,
// so common words are never mangled (unlike a strict snippet replacement).
// Only writes when the user clicks Add.
async function maybeOfferCorrection(id: string, oldText: string, newText: string) {
  const corr = await invoke<Correction | null>("propose_correction", { old: oldText, new: newText });
  if (!corr || corr.replace.trim() === "") return;

  const card = document.querySelector<HTMLElement>(`.feed-item[data-id="${id}"]`);
  if (!card) return;

  const panel = document.createElement("div");
  panel.className = "learn-panel";

  const label = document.createElement("span");
  label.className = "learn-panel-label";
  label.textContent = "Remember this spelling?";

  const termInput = document.createElement("input");
  termInput.className = "learn-panel-input";
  termInput.value = corr.replace;

  const addBtn = document.createElement("button");
  addBtn.className = "btn-primary";
  addBtn.textContent = "Add to Dictionary";

  const dismissBtn = document.createElement("button");
  dismissBtn.className = "btn-secondary";
  dismissBtn.textContent = "Dismiss";

  const note = document.createElement("span");
  note.className = "learn-panel-note";
  note.textContent = "Biases the engine toward this spelling — never rewrites your text.";

  addBtn.onclick = async () => {
    const term = termInput.value.trim();
    if (term === "") {
      label.textContent = "Enter a term first.";
      return;
    }
    try {
      await invoke("add_vocabulary_hint", { word: term });
      loadDictionary(); // refresh the Dictionary tab so the hint shows immediately
      panel.innerHTML = "";
      const done = document.createElement("span");
      done.className = "learn-panel-label";
      done.textContent = "Added to Dictionary ✓";
      panel.appendChild(done);
      setTimeout(() => panel.remove(), 2000);
    } catch (err) {
      label.textContent = String(err);
    }
  };

  dismissBtn.onclick = () => panel.remove();

  panel.appendChild(label);
  panel.appendChild(termInput);
  panel.appendChild(addBtn);
  panel.appendChild(dismissBtn);
  panel.appendChild(note);

  card.insertAdjacentElement("afterend", panel);
}

// Flip a history card into an inline editor. Save persists via
// update_transcription and re-renders; Cancel restores the read-only view.
function enterEditMode(card: HTMLElement, item: TranscriptionItem) {
  card.classList.add("editing");
  card.innerHTML = "";

  const ta = document.createElement("textarea");
  ta.className = "feed-item-editor";
  ta.value = item.text;

  const editActions = document.createElement("div");
  editActions.className = "feed-item-edit-actions";

  const saveBtn = document.createElement("button");
  saveBtn.className = "btn-primary";
  saveBtn.textContent = "Save";

  const cancelBtn = document.createElement("button");
  cancelBtn.className = "btn-secondary";
  cancelBtn.textContent = "Cancel";

  editActions.appendChild(saveBtn);
  editActions.appendChild(cancelBtn);
  card.appendChild(ta);
  card.appendChild(editActions);
  ta.focus();

  cancelBtn.onclick = () => loadHistory(false);

  saveBtn.onclick = async () => {
    const newText = ta.value.trim();
    const oldText = item.text;
    if (newText === oldText || newText === "") {
      loadHistory(false);
      return;
    }
    try {
      await invoke("update_transcription", { id: item.id, text: newText });
      item.text = newText;
      item.word_count = newText.split(/\s+/).filter(Boolean).length;
      loadHistory(false);
      maybeOfferCorrection(item.id, oldText, newText);
    } catch (err) {
      console.error(err);
    }
  };
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

// Escape the text for safe innerHTML, then wrap case-insensitive matches of
// `query` in a highlight span. Query regex-special chars are escaped.
function highlightMatches(text: string, query: string): string {
  const esc = escapeHtml(text);
  if (!query) return esc;
  const q = query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return esc.replace(new RegExp(`(${q})`, "gi"), '<mark class="search-hl">$1</mark>');
}

async function loadHistory(forceFetch = true) {
  if (forceFetch || !cachedHistory) {
    cachedHistory = await invoke<History>("get_history");
    visibleHistoryCount = 50;
  }

  const history = cachedHistory;

  let totalWords = 0;
  let totalChars = 0;
  let totalDuration = 0;

  transcriptionFeed.innerHTML = "";

  if (forceFetch) {
    transcriptionFeed.scrollTop = 0;
  }

  if (history.items.length === 0) {
    transcriptionFeed.innerHTML = '<div style="color: var(--text-tertiary); font-size: 13px; text-align: center; padding: 20px;">No transcriptions yet.</div>';
    statWords.textContent = "0";
    statWpm.textContent = "0";
    statCount.textContent = "0";
    return;
  }

  // Apply the search filter (case-insensitive substring); everything below
  // operates on the filtered set so stats/count/pagination reflect it.
  const query = historyQuery.trim().toLowerCase();
  const filtered = query
    ? history.items.filter(it => it.text.toLowerCase().includes(query))
    : history.items;

  if (filtered.length === 0) {
    transcriptionFeed.innerHTML = `<div style="color: var(--text-tertiary); font-size: 13px; text-align: center; padding: 20px;">No transcriptions match &ldquo;${escapeHtml(historyQuery.trim())}&rdquo;.</div>`;
    statWords.textContent = "0";
    statWpm.textContent = "0";
    statCount.textContent = "0";
    return;
  }

  // Calculate statistics over the filtered set
  filtered.forEach(item => {
    totalWords += item.word_count;
    totalChars += item.text.length;
    totalDuration += item.duration_secs;
  });

  // Group and render only the visible subset of items
  const itemsToRender = filtered.slice(0, visibleHistoryCount);
  const groups = new Map<string, typeof history.items>();
  
  itemsToRender.forEach(item => {
    const date = new Date(item.timestamp * 1000);
    const today = new Date();
    const yesterday = new Date(today);
    yesterday.setDate(yesterday.getDate() - 1);
    
    let groupKey = "";
    if (date.toDateString() === today.toDateString()) {
      groupKey = "Today";
    } else if (date.toDateString() === yesterday.toDateString()) {
      groupKey = "Yesterday";
    } else {
      groupKey = date.toLocaleDateString(undefined, { weekday: 'long', month: 'short', day: 'numeric', year: 'numeric' });
    }
    
    if (!groups.has(groupKey)) {
      groups.set(groupKey, []);
    }
    groups.get(groupKey)!.push(item);
  });

  for (const [groupName, items] of groups.entries()) {
    const header = document.createElement("div");
    header.style.cssText = "font-size: 12px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.5px; color: var(--text); margin: 16px 0 6px 4px;";
    header.textContent = groupName;
    transcriptionFeed.appendChild(header);

    items.forEach(item => {
      const date = new Date(item.timestamp * 1000);
      const timeStr = date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

      const el = document.createElement("div");
      el.className = "feed-item";
      el.dataset.id = item.id;

      const timeEl = document.createElement("div");
      timeEl.className = "feed-item-time";
      timeEl.textContent = timeStr;

      const textEl = document.createElement("div");
      textEl.className = "feed-item-text";
      if (query) {
        textEl.innerHTML = highlightMatches(item.text, query);
      } else {
        textEl.textContent = item.text;
      }

      const actions = document.createElement("div");
      actions.className = "feed-item-actions";

      const copyBtn = document.createElement("button");
      copyBtn.className = "btn-primary feed-item-copy-btn";
      copyBtn.textContent = "Copy";
      copyBtn.onclick = () => {
        navigator.clipboard.writeText(item.text);
        copyBtn.textContent = "Copied";
        setTimeout(() => copyBtn.textContent = "Copy", 2000);
      };

      const editBtn = document.createElement("button");
      editBtn.className = "btn-secondary feed-item-edit-btn";
      editBtn.textContent = "Edit";
      editBtn.onclick = () => enterEditMode(el, item);

      const delBtn = document.createElement("button");
      delBtn.className = "btn-secondary feed-item-del-btn";
      delBtn.textContent = "Delete";
      delBtn.onclick = () => {
        const items2 = cachedHistory!.items;
        const idx = items2.findIndex(i => i.id === item.id);
        if (idx === -1) return;
        const [removed] = items2.splice(idx, 1);
        loadHistory(false);
        showUndoToast(
          "Transcription deleted",
          () => {
            cachedHistory!.items.splice(idx, 0, removed);
            loadHistory(false);
          },
          () => {
            invoke("delete_transcription", { id: removed.id }).catch(err => console.error(err));
          },
        );
      };

      actions.appendChild(copyBtn);
      actions.appendChild(editBtn);
      actions.appendChild(delBtn);

      el.appendChild(timeEl);
      el.appendChild(textEl);
      el.appendChild(actions);
      transcriptionFeed.appendChild(el);
    });
  }

  // Render a clean pagination button if there are more items remaining
  if (filtered.length > visibleHistoryCount) {
    const loadMoreBtn = document.createElement("button");
    loadMoreBtn.className = "load-more-btn";
    loadMoreBtn.textContent = "Load Older Transcriptions";
    
    loadMoreBtn.addEventListener("click", () => {
      visibleHistoryCount += 50;
      loadHistory(false); // Quick render from cache without another Tauri IPC call!
    });
    
    transcriptionFeed.appendChild(loadMoreBtn);
  }

  statWords.textContent = totalWords.toLocaleString();
  statCount.textContent = filtered.length.toLocaleString();
  
  // Standard calculation: (Characters / 5) / (Time in minutes)
  const wpm = totalDuration > 0 ? Math.round((totalChars / 5) / (totalDuration / 60)) : 0;
  statWpm.textContent = wpm.toString();
}

// ── Custom Vocabulary & Text Replacements ───────────────────

interface ReplacementEntry {
  find: string;
  replace: string;
  case_sensitive: boolean;
}

interface DictionaryData {
  vocabulary_hints: string[];
  replacements: ReplacementEntry[];
}

// Spelling Hints selectors
const dictHintWordInput = document.getElementById("dict-hint-word") as HTMLInputElement;
const dictHintAddBtn = document.getElementById("dict-hint-add-btn")!;
const dictHintsList = document.getElementById("dict-hints-list")!;

// Text Replacements selectors
const replaceFindInput = document.getElementById("replace-find") as HTMLInputElement;
const replaceWithInput = document.getElementById("replace-with") as HTMLInputElement;
const replaceCaseCheckbox = document.getElementById("replace-case") as HTMLInputElement;
const replaceAddBtn = document.getElementById("replace-add-btn")!;
const dictReplacementsList = document.getElementById("dict-replacements-list")!;

async function loadDictionary() {
  const data = await invoke<DictionaryData>("get_dictionary");
  
  // 1. Render Spelling Hints List
  dictHintsList.innerHTML = "";
  const hints = data.vocabulary_hints || [];
  
  if (hints.length === 0) {
    dictHintsList.innerHTML = '<div style="color: var(--text-tertiary); font-size: 13px; text-align: center; padding: 20px;">No spelling hints added yet.</div>';
  } else {
    hints.forEach((word, index) => {
      const row = document.createElement("div");
      row.className = "dict-entry";

      const wordSpan = document.createElement("span");
      wordSpan.className = "dict-entry-word";
      wordSpan.textContent = word;

      const actions = document.createElement("div");
      actions.className = "dict-entry-actions";

      const deleteBtn = document.createElement("button");
      deleteBtn.className = "dict-btn dict-btn-delete";
      deleteBtn.textContent = "Delete";
      deleteBtn.onclick = async () => {
        await invoke("remove_vocabulary_hint", { index });
        loadDictionary();
      };

      actions.appendChild(deleteBtn);
      row.appendChild(wordSpan);
      row.appendChild(actions);
      dictHintsList.appendChild(row);
    });
  }

  // 2. Render Text Replacements List
  dictReplacementsList.innerHTML = "";
  const replacements = data.replacements || [];
  
  replacements.forEach((entry, index) => {
    const row = document.createElement("div");
    row.className = "replacement-row";

    const findSpan = document.createElement("div");
    findSpan.className = "col-find";
    findSpan.textContent = entry.find;

    const arrowDiv = document.createElement("div");
    arrowDiv.className = "col-arrow";
    arrowDiv.innerHTML = "&rarr;";

    const replaceSpan = document.createElement("div");
    replaceSpan.className = "col-replace";
    replaceSpan.textContent = entry.replace;

    const optsSpan = document.createElement("div");
    optsSpan.className = "col-opts";
    optsSpan.textContent = entry.case_sensitive ? "Case Match" : "Fuzzy Case";

    const actions = document.createElement("div");
    actions.className = "col-action";

    const deleteBtn = document.createElement("button");
    deleteBtn.className = "dict-btn dict-btn-delete";
    deleteBtn.textContent = "Delete";
    deleteBtn.onclick = async () => {
      await invoke("remove_replacement", { index });
      loadDictionary();
    };

    actions.appendChild(deleteBtn);
    row.appendChild(findSpan);
    row.appendChild(arrowDiv);
    row.appendChild(replaceSpan);
    row.appendChild(optsSpan);
    row.appendChild(actions);
    dictReplacementsList.appendChild(row);
  });
}

// Add Spelling Hint Handler
dictHintAddBtn.addEventListener("click", async () => {
  const word = dictHintWordInput.value.trim();
  if (!word) return;
  await invoke("add_vocabulary_hint", { word });
  dictHintWordInput.value = "";
  loadDictionary();
});

dictHintWordInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") dictHintAddBtn.click();
});

// Add Text Replacement Handler
replaceAddBtn.addEventListener("click", async () => {
  const find = replaceFindInput.value.trim();
  const replace = replaceWithInput.value.trim();
  if (!find) return;
  
  const caseSensitive = replaceCaseCheckbox.checked;
  await invoke("add_replacement", { find, replace, caseSensitive });
  
  replaceFindInput.value = "";
  replaceWithInput.value = "";
  replaceCaseCheckbox.checked = false;
  loadDictionary();
});

// Allow Enter keys on replacements inputs to submit
replaceFindInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") replaceWithInput.focus();
});

replaceWithInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") replaceAddBtn.click();
});

// ── App context rules (Auto-context per-app overrides) ───────

const CATEGORY_LABELS: Record<string, string> = {
  developer: "Developer",
  email: "Email",
  messaging: "Messaging",
  professional: "Professional",
  general: "General",
};

async function loadAppRules() {
  const rules = await invoke<AppRule[]>("get_app_rules");
  // Keep the local copy in sync: saveSettings() sends the whole Settings
  // object, so a stale appRules array would clobber backend-added rules.
  currentSettings.appRules = rules;
  appRulesList.innerHTML = "";

  if (rules.length === 0) {
    appRulesList.innerHTML = '<div style="color: var(--text-tertiary); font-size: 13px; text-align: center; padding: 20px;">No app rules yet.</div>';
    return;
  }

  rules.forEach((rule, index) => {
    const row = document.createElement("div");
    row.className = "dict-entry";

    const wordSpan = document.createElement("span");
    wordSpan.className = "dict-entry-word";
    const label = CATEGORY_LABELS[rule.category] || rule.category;
    // A rule may be app-only, app+title, or title-only (blank process → any app).
    let desc: string;
    if (rule.process_name && rule.title_contains) {
      desc = `${rule.process_name} · title contains "${rule.title_contains}"`;
    } else if (rule.process_name) {
      desc = rule.process_name;
    } else {
      desc = `Any app · title contains "${rule.title_contains}"`;
    }
    wordSpan.textContent = `${desc} → ${label}`;

    const actions = document.createElement("div");
    actions.className = "dict-entry-actions";

    const deleteBtn = document.createElement("button");
    deleteBtn.className = "dict-btn dict-btn-delete";
    deleteBtn.textContent = "Delete";
    deleteBtn.onclick = async () => {
      await invoke("remove_app_rule", { index });
      loadAppRules();
    };

    actions.appendChild(deleteBtn);
    row.appendChild(wordSpan);
    row.appendChild(actions);
    appRulesList.appendChild(row);
  });
}

// Add App Rule Handler
appRuleAddBtn.addEventListener("click", async () => {
  const processName = appRuleProcess.value.trim();
  const titleRaw = appRuleTitle.value.trim();
  if (!processName && !titleRaw) {
    showToast("Pick an app or enter a title filter first.");
    return;
  }
  try {
    await invoke("add_app_rule", {
      processName,
      titleContains: titleRaw === "" ? null : titleRaw,
      category: appRuleCategory.value,
    });
  } catch (e) {
    showToast(String(e));
    return;
  }
  appRuleProcess.value = "";
  appRuleTitle.value = "";
  closePicker();
  loadAppRules();
});

// Custom in-app dropdown for the app-rule process picker. Matches the app's dark surfaces
// instead of the OS-drawn datalist, and shows friendly names with the process name muted.
let runningAppsCache: RunningApp[] = [];
let pickerActiveIndex = -1; // index into the currently-rendered (filtered) items, -1 = none

function isPickerOpen(): boolean {
  return !appPickerPanel.classList.contains("hidden");
}

function closePicker() {
  appPickerPanel.classList.add("hidden");
  appPickerPanel.innerHTML = "";
  pickerActiveIndex = -1;
}

// Render the panel filtered by the current input value (case-insensitive substring against
// both display_name and process_name; empty input shows all). Empty result → hide entirely.
function renderPicker() {
  const query = appRuleProcess.value.trim().toLowerCase();
  const filtered = query === ""
    ? runningAppsCache
    : runningAppsCache.filter(
        (a) =>
          a.display_name.toLowerCase().includes(query) ||
          a.process_name.toLowerCase().includes(query),
      );

  appPickerPanel.innerHTML = "";
  if (filtered.length === 0) {
    closePicker();
    return;
  }

  pickerActiveIndex = -1;
  filtered.forEach((app) => {
    const item = document.createElement("div");
    item.className = "app-picker-item";

    const name = document.createElement("div");
    name.className = "app-picker-item-name";
    name.textContent = app.display_name;

    const proc = document.createElement("div");
    proc.className = "app-picker-item-proc";
    proc.textContent = app.process_name;

    item.appendChild(name);
    item.appendChild(proc);

    // mousedown + preventDefault so clicking an item doesn't blur-close the input first.
    item.addEventListener("mousedown", (e) => {
      e.preventDefault();
      appRuleProcess.value = app.process_name;
      closePicker();
    });

    appPickerPanel.appendChild(item);
  });

  appPickerPanel.classList.remove("hidden");
}

// Highlight the item at pickerActiveIndex and scroll it into view.
function setPickerActive(index: number) {
  const items = Array.from(appPickerPanel.children) as HTMLElement[];
  if (items.length === 0) return;
  items.forEach((el) => el.classList.remove("active"));
  pickerActiveIndex = ((index % items.length) + items.length) % items.length;
  const el = items[pickerActiveIndex];
  el.classList.add("active");
  el.scrollIntoView({ block: "nearest" });
}

// Fetch on focus so the picker reflects what's running right now each time it opens.
// Fails silently (empty list) so a backend hiccup never toasts or blocks free-text typing.
appRuleProcess.addEventListener("focus", async () => {
  try {
    runningAppsCache = await invoke<RunningApp[]>("list_running_apps");
  } catch {
    runningAppsCache = [];
  }
  renderPicker();
});

appRuleProcess.addEventListener("input", () => {
  renderPicker();
});

appRuleProcess.addEventListener("keydown", (e) => {
  if (isPickerOpen()) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setPickerActive(pickerActiveIndex + 1);
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      setPickerActive(pickerActiveIndex - 1);
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      closePicker();
      return;
    }
    if (e.key === "Enter" && pickerActiveIndex >= 0) {
      // Panel open with a highlight → select it; do NOT also trigger Add.
      e.preventDefault();
      const items = Array.from(appPickerPanel.children) as HTMLElement[];
      const proc = items[pickerActiveIndex].querySelector(".app-picker-item-proc");
      if (proc && proc.textContent) appRuleProcess.value = proc.textContent;
      closePicker();
      return;
    }
  }
  // Existing behavior: Enter (no highlighted item) triggers Add Rule.
  if (e.key === "Enter") appRuleAddBtn.click();
});

// Close on click-outside. A mousedown inside the panel is handled with preventDefault on
// items, so the input keeps focus and the selection lands before this fires.
document.addEventListener("mousedown", (e) => {
  const target = e.target as Node;
  if (!appPickerPanel.contains(target) && target !== appRuleProcess) {
    closePicker();
  }
});

// ---------------------------------------------------------------------------
// In-app updates
//
// The updater compares this build's version against latest.json on the GitHub release and,
// if newer, downloads the signed NSIS installer and runs it in passive mode (a progress bar,
// no wizard to click through). Windows terminates the app as the installer takes over, so
// the relaunch below only matters if that ever stops being true.
//
// A failed check is deliberately quiet on startup — no network, a rate-limited GitHub, or a
// release without latest.json must never greet someone with an error they cannot act on.
// The same failure IS reported when the check was asked for by the button.
// ---------------------------------------------------------------------------
const updateBtn = document.getElementById("update-btn") as HTMLButtonElement;
const updateStatus = document.getElementById("update-status")!;
const appVersion = document.getElementById("app-version")!;
const updateProgress = document.getElementById("update-progress")!;
const updateProgressFill = document.getElementById("update-progress-fill")!;

/// The update found by the last check, held so the button can install it on the next click
/// without re-querying GitHub.
let pendingUpdate: Update | null = null;

async function installUpdate(update: Update) {
  updateBtn.disabled = true;
  updateBtn.textContent = "Downloading…";
  updateProgress.classList.remove("hidden");
  updateProgressFill.style.width = "0%";

  // contentLength arrives on Started; Progress events carry only the size of each chunk, so
  // the total has to be accumulated here to show a percentage.
  let total = 0;
  let received = 0;
  try {
    await update.downloadAndInstall((event) => {
      switch (event.event) {
        case "Started":
          total = event.data.contentLength ?? 0;
          break;
        case "Progress":
          received += event.data.chunkLength;
          if (total > 0) {
            updateProgressFill.style.width = `${Math.min(100, (received / total) * 100)}%`;
          }
          break;
        case "Finished":
          updateProgressFill.style.width = "100%";
          updateStatus.textContent = "Installing… Typr will restart.";
          break;
      }
    });
    await relaunch();
  } catch (e) {
    updateProgress.classList.add("hidden");
    updateStatus.textContent = `Update failed: ${String(e)}`;
    updateBtn.disabled = false;
    updateBtn.textContent = "Retry";
  }
}

async function runUpdateCheck(userAsked: boolean) {
  if (pendingUpdate) {
    await installUpdate(pendingUpdate);
    return;
  }
  if (userAsked) {
    updateBtn.disabled = true;
    updateStatus.textContent = "Checking…";
  }
  try {
    const update = await checkForUpdate();
    if (update) {
      pendingUpdate = update;
      updateStatus.textContent = `Version ${update.version} is available.`;
      updateBtn.textContent = "Download & install";
      updateBtn.disabled = false;
    } else if (userAsked) {
      updateStatus.textContent = "You're on the latest version.";
      updateBtn.disabled = false;
    }
  } catch (e) {
    if (userAsked) {
      updateStatus.textContent = `Could not check for updates: ${String(e)}`;
      updateBtn.disabled = false;
    } else {
      console.warn("Background update check failed:", e);
    }
  }
}

updateBtn.addEventListener("click", () => runUpdateCheck(true));

// Initialize
loadSettings();
getVersion()
  .then((v) => {
    appVersion.textContent = v;
  })
  .catch(() => {});
runUpdateCheck(false);
