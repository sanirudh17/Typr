import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface VocabularyEntry {
  id: string; canonical_term: string; aliases: string[];
  category: "person" | "product" | "technical" | "acronym" | "other";
  notes: string; hit_count: number; source: "manual" | "learned";
  confidence: number; enabled: boolean; created_at: number; updated_at: number;
}
interface LearningSettings {
  learnFromCorrections: boolean; dictionaryCloudHints: boolean;
  learningObservationSeconds: number; learningSuggestionSeconds: number; learningMaxPhraseChars: number;
}
let entries: VocabularyEntry[] = [];
let editing: VocabularyEntry | undefined;
let hooks: { settings: () => LearningSettings; save: () => Promise<void>; toast: (text: string) => void; reload: () => Promise<void> };
const input = (id: string) => document.getElementById(id) as HTMLInputElement;
function blank(): VocabularyEntry {
  return { id: "", canonical_term: "", aliases: [], category: "other", notes: "", hit_count: 0, source: "manual", confidence: 1, enabled: true, created_at: 0, updated_at: 0 };
}
function focusEntry(id?: string) {
  const summary = [...document.querySelectorAll<HTMLElement>(".dictionary-entry")]
    .find(row => row.dataset.entryId === id)?.querySelector<HTMLElement>("summary");
  (summary ?? document.getElementById("dictionary-new"))?.focus();
}
function resetEditor(restoreFocus = false) {
  const previous = editing?.id;
  editing = undefined;
  (document.getElementById("dictionary-form") as HTMLFormElement).reset();
  document.querySelector<HTMLDetailsElement>(".dictionary-form-details")!.open = false;
  document.getElementById("dict-hint-add-btn")!.textContent = "Add term";
  document.getElementById("dictionary-editor")!.classList.add("hidden");
  document.getElementById("dictionary-error")!.textContent = "";
  const select = document.getElementById("dictionary-category") as HTMLSelectElement;
  select.value = "other";
  select.dispatchEvent(new Event("change"));
  if (restoreFocus) focusEntry(previous);
}
function edit(entry: VocabularyEntry) {
  document.getElementById("dictionary-editor")!.classList.remove("hidden");
  editing = entry;
  input("dict-hint-word").value = entry.canonical_term;
  input("dictionary-aliases").value = entry.aliases.join(", ");
  input("dictionary-notes").value = entry.notes;
  const select = document.getElementById("dictionary-category") as HTMLSelectElement;
  select.value = entry.category; select.dispatchEvent(new Event("change"));
  document.getElementById("dict-hint-add-btn")!.textContent = "Save changes";
  input("dict-hint-word").focus();
  document.getElementById("dictionary-form")!.scrollIntoView({ block: "nearest", behavior: "smooth" });
}
export function renderVocabulary(data: VocabularyEntry[]) {
  entries = data;
  document.getElementById("dictionary-count")!.textContent = `${entries.length} saved term${entries.length === 1 ? "" : "s"}`;
  const list = document.getElementById("dict-hints-list")!;
  const focusedId = (document.activeElement?.closest(".dictionary-entry") as HTMLElement | null)?.dataset.entryId;
  const expanded = new Set([...list.querySelectorAll<HTMLDetailsElement>("details[open]")].map(d => d.parentElement?.dataset.entryId));
  list.replaceChildren();
  const query = input("dictionary-search").value.toLowerCase().trim();
  const filtered = entries.filter(e => [e.canonical_term, ...e.aliases, e.category, e.notes].join(" ").toLowerCase().includes(query));
  if (!filtered.length) {
    const empty = document.createElement("p"); empty.className = "dictionary-empty";
    empty.textContent = query ? "No matching terms. Try another search." : "Your remembered names will appear here. Add a term to get started.";
    list.append(empty); if (focusedId) focusEntry(); return;
  }
  for (const e of filtered) {
    const row = document.createElement("div"); row.className = "dictionary-entry"; row.dataset.entryId = e.id;
    if (!e.enabled) row.classList.add("dictionary-entry-disabled");
    const details = document.createElement("details"); details.className = "dictionary-entry-details";
    details.open = expanded.has(e.id);
    const summary = document.createElement("summary");
    const title = document.createElement("strong"); title.textContent = e.canonical_term;
    summary.append(title);
    const meta = document.createElement("span"); meta.className = "dictionary-entry-meta";
    meta.textContent = `${e.category} · ${e.source} · ${e.hit_count} dictation${e.hit_count === 1 ? "" : "s"}`;
    details.append(summary,meta);
    if (e.notes) { const n = document.createElement("p"); n.textContent = e.notes; details.append(n); }
    const actions = document.createElement("div"); actions.className = "dictionary-entry-actions";
    const enabled = document.createElement("button"); enabled.className = "btn-secondary"; enabled.textContent = e.enabled ? "Enabled" : "Disabled";
    enabled.setAttribute("aria-pressed", String(e.enabled)); enabled.setAttribute("aria-label", `${e.enabled ? "Disable" : "Enable"} ${e.canonical_term}`);
    enabled.onclick = async () => { summary.focus(); enabled.disabled = true; try { await invoke("save_dictionary_entry", { entry: { ...e, enabled: !e.enabled } }); await hooks.reload(); } catch (err) { hooks.toast(String(err)); } finally { enabled.disabled = false; focusEntry(e.id); } };
    const editBtn = document.createElement("button"); editBtn.className = "btn-secondary"; editBtn.textContent = "Edit"; editBtn.onclick = () => edit(e);
    const del = document.createElement("button"); del.className = "dict-btn dict-btn-delete"; del.textContent = "Delete";
    del.onclick = async () => { summary.focus(); del.disabled = true; try { await invoke("delete_dictionary_entry", { id: e.id }); if (editing?.id === e.id) resetEditor(); await hooks.reload(); focusEntry(); } catch (err) { hooks.toast(String(err)); del.disabled = false; focusEntry(e.id); } };
    actions.append(enabled, editBtn, del); row.append(details, actions); list.append(row);
  }
  if (focusedId) focusEntry(focusedId);
}
export function syncLearningSettings() {
  const settings = hooks.settings();
  settings.learnFromCorrections = true;
  settings.dictionaryCloudHints = true;
  input("learning-observation").value = String(settings.learningObservationSeconds ?? 60);
  input("learning-suggestion").value = String(settings.learningSuggestionSeconds ?? 10);
  input("learning-phrase").value = String(settings.learningMaxPhraseChars ?? 12);
}
export function initDictionaryFeature(options: typeof hooks) {
  hooks = options;
  input("dictionary-search").oninput = () => renderVocabulary(entries);
  document.getElementById("dictionary-new")!.onclick = () => {
    resetEditor(); document.getElementById("dictionary-editor")!.classList.remove("hidden"); input("dict-hint-word").focus();
  };
  document.getElementById("dictionary-cancel")!.onclick = () => resetEditor(true);
  (document.getElementById("dictionary-form") as HTMLFormElement).onsubmit = async event => {
    event.preventDefault(); const button = document.getElementById("dict-hint-add-btn") as HTMLButtonElement;
    button.disabled = true;
    try {
      const entry = { ...(editing ?? blank()), canonical_term: input("dict-hint-word").value.trim(), aliases: input("dictionary-aliases").value.split(",").map(s => s.trim()).filter(Boolean), category: (document.getElementById("dictionary-category") as HTMLSelectElement).value, notes: input("dictionary-notes").value.trim() };
      await invoke("save_dictionary_entry", { entry }); resetEditor(); await hooks.reload(); focusEntry(entry.id); hooks.toast("Dictionary saved");
    } catch (err) { document.getElementById("dictionary-error")!.textContent = String(err); } finally { button.disabled = false; }
  };
  for (const id of ["learning-observation", "learning-suggestion", "learning-phrase"]) {
    input(id).onchange = async () => {
      const settings = hooks.settings(); const previous = { ...settings };
      const value = (key: string, min: number, max: number) => Math.round(Math.max(min, Math.min(max, Number(input(key).value) || min)));
      settings.learnFromCorrections = true;
      settings.dictionaryCloudHints = true;
      settings.learningObservationSeconds = value("learning-observation", 10, 60);
      settings.learningSuggestionSeconds = value("learning-suggestion", 5, 60);
      settings.learningMaxPhraseChars = value("learning-phrase", 2, 32);
      try { await hooks.save(); } catch (err) { Object.assign(settings, previous); hooks.toast(String(err)); }
      syncLearningSettings();
    };
  }
  void listen("dictionary-updated", () => { void hooks.reload(); });
}
