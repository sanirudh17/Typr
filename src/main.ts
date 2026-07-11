import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface Settings {
  microphone: string;
  engine: string;
  whisperModel: string;
  groqApiKey: string;
  recordingMode: string;
  hotkey: string;
  cloudModel: string;
  aiEnabled: boolean;
  aiModel: string;
  aiProfile: string;
  aiPromptFormat: string;
  aiTone: string;
  aiFormat: string;
  aiCustomInstructions: string;
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
const localSettings = document.getElementById("local-settings")!;
const cloudSettings = document.getElementById("cloud-settings")!;
const modelSelect = document.getElementById("model-select") as HTMLSelectElement;
const downloadBtn = document.getElementById("download-btn")!;
const downloadProgress = document.getElementById("download-progress")!;
const progressFill = document.getElementById("progress-fill")!;
const groqKey = document.getElementById("groq-key") as HTMLInputElement;
const cloudModelSettings = document.getElementById("cloud-model-settings")!;
const modelFast = document.getElementById("model-fast")!;
const modelAccurate = document.getElementById("model-accurate")!;
const aiOff = document.getElementById("ai-off")!;
const aiOn = document.getElementById("ai-on")!;
const aiModelSelect = document.getElementById("ai-model-select") as HTMLSelectElement;
const aiProfileCleanup = document.getElementById("ai-profile-cleanup")!;
const aiProfilePrompt = document.getElementById("ai-profile-prompt")!;
const aiProfileAuto = document.getElementById("ai-profile-auto")!;
const aiFormatSettings = document.getElementById("ai-format-settings")!;
const aiToneDefault = document.getElementById("ai-tone-default")!;
const aiToneFormal = document.getElementById("ai-tone-formal")!;
const aiToneCasual = document.getElementById("ai-tone-casual")!;
const aiToneConcise = document.getElementById("ai-tone-concise")!;
const aiStyleDefault = document.getElementById("ai-style-default")!;
const aiStyleBullets = document.getElementById("ai-style-bullets")!;
const aiStyleParagraphs = document.getElementById("ai-style-paragraphs")!;
const aiStyleRaw = document.getElementById("ai-style-raw")!;
const aiCustomInstructions = document.getElementById("ai-custom-instructions") as HTMLTextAreaElement;
const aiFormatNatural = document.getElementById("ai-format-natural")!;
const aiFormatStructured = document.getElementById("ai-format-structured")!;
const modeToggle = document.getElementById("mode-toggle")!;
const modePtt = document.getElementById("mode-ptt")!;
const hotkeyText = document.getElementById("hotkey-text")!;
const statCount = document.getElementById("stat-count")!;
const statWords = document.getElementById("stat-words")!;
const statWpm = document.getElementById("stat-wpm")!;
const transcriptionFeed = document.getElementById("transcription-feed")!;

// Section navigation
const navItems = document.querySelectorAll(".nav-item");
const sections = document.querySelectorAll(".content-section");

navItems.forEach((item) => {
  item.addEventListener("click", () => {
    const target = item.getAttribute("data-section");
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

  // Groq key
  groqKey.value = currentSettings.groqApiKey;

  // Cloud model
  setCloudModel(currentSettings.cloudModel || "accurate");

  // Recording mode
  setRecordingMode(currentSettings.recordingMode);

  // Hotkey
  hotkeyText.textContent = currentSettings.hotkey.replace("CmdOrCtrl", "Cmd");

  // AI post-processing
  setAiEnabled(currentSettings.aiEnabled);
  setAiModel(currentSettings.aiModel || "openai/gpt-oss-20b");
  setAiProfile(currentSettings.aiProfile || "cleanup");
  setAiPromptFormat(currentSettings.aiPromptFormat || "natural");
  setAiTone(currentSettings.aiTone || "default");
  setAiFormat(currentSettings.aiFormat || "default");
  aiCustomInstructions.value = currentSettings.aiCustomInstructions || "";
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

function setEngine(engine: string) {
  currentSettings.engine = engine;
  engineLocal.classList.toggle("active", engine === "local");
  engineCloud.classList.toggle("active", engine === "cloud");
  localSettings.classList.toggle("hidden", engine !== "local");
  cloudSettings.classList.toggle("hidden", engine !== "cloud");
  cloudModelSettings.classList.toggle("hidden", engine !== "cloud");
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
}

function setAiModel(model: string) {
  const m = model === "openai/gpt-oss-120b" ? "openai/gpt-oss-120b" : "openai/gpt-oss-20b";
  currentSettings.aiModel = m;
  aiModelSelect.value = m;
}

function setAiProfile(profile: string) {
  const p = profile === "prompt" ? "prompt" : profile === "auto" ? "auto" : "cleanup";
  currentSettings.aiProfile = p;
  aiProfileCleanup.classList.toggle("active", p === "cleanup");
  aiProfilePrompt.classList.toggle("active", p === "prompt");
  aiProfileAuto.classList.toggle("active", p === "auto");
  // Format sub-selector only applies to Prompt Mode.
  aiFormatSettings.classList.toggle("hidden", p !== "prompt");
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

function setRecordingMode(mode: string) {
  currentSettings.recordingMode = mode;
  modeToggle.classList.toggle("active", mode === "toggle");
  modePtt.classList.toggle("active", mode === "push-to-talk");
}

async function checkModelStatus() {
  const downloaded = await invoke<boolean>("check_model_downloaded", {
    modelSize: modelSelect.value,
  });
  downloadBtn.textContent = downloaded ? "\u2713" : "Download";
  (downloadBtn as HTMLButtonElement).disabled = downloaded;
}

async function saveSettings() {
  currentSettings.microphone = micSelect.value;
  currentSettings.whisperModel = modelSelect.value;
  currentSettings.groqApiKey = groqKey.value;
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
  downloadProgress.classList.remove("hidden");
  progressFill.style.width = "0%";

  try {
    await invoke("download_model", { modelSize: modelSelect.value });
    downloadBtn.textContent = "\u2713";
  } catch (e) {
    downloadBtn.textContent = "Retry";
    (downloadBtn as HTMLButtonElement).disabled = false;
    console.error("Download failed:", e);
  }
  downloadProgress.classList.add("hidden");
});

groqKey.addEventListener("change", () => saveSettings());

aiOff.addEventListener("click", () => {
  setAiEnabled(false);
  saveSettings();
});

aiOn.addEventListener("click", () => {
  setAiEnabled(true);
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

// Listen for download progress
listen<DownloadProgress>("download-progress", (event) => {
  const { percent } = event.payload;
  progressFill.style.width = `${percent}%`;
});

// Listen for history updates
listen("history-updated", () => {
  loadHistory();
});

let visibleHistoryCount = 50;
let cachedHistory: History | null = null;

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

  // Calculate statistics over the entire history
  history.items.forEach(item => {
    totalWords += item.word_count;
    totalChars += item.text.length;
    totalDuration += item.duration_secs;
  });

  // Group and render only the visible subset of items
  const itemsToRender = history.items.slice(0, visibleHistoryCount);
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
      
      const timeEl = document.createElement("div");
      timeEl.className = "feed-item-time";
      timeEl.textContent = timeStr;

      const textEl = document.createElement("div");
      textEl.className = "feed-item-text";
      textEl.textContent = item.text;

      const copyBtn = document.createElement("button");
      copyBtn.className = "btn-primary feed-item-copy-btn";
      copyBtn.textContent = "Copy";
      copyBtn.onclick = () => {
        navigator.clipboard.writeText(item.text);
        copyBtn.textContent = "Copied";
        setTimeout(() => copyBtn.textContent = "Copy", 2000);
      };

      el.appendChild(timeEl);
      el.appendChild(textEl);
      el.appendChild(copyBtn);
      transcriptionFeed.appendChild(el);
    });
  }

  // Render a clean pagination button if there are more items remaining
  if (history.items.length > visibleHistoryCount) {
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
  statCount.textContent = history.items.length.toLocaleString();
  
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

// Initialize
loadSettings();
