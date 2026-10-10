import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
type Suggestion = { id: string; candidate: { wrong: string; correct: string }; created_at: number; expires_at: number };
let pending: Suggestion | null = null;
const accept = document.getElementById("learning-accept") as HTMLButtonElement;
const dismiss = document.getElementById("learning-dismiss") as HTMLButtonElement;
const scheme = matchMedia("(prefers-color-scheme: dark)");
let preferredTheme = (window as unknown as { __LEARNING_THEME__: string }).__LEARNING_THEME__ || "system";
function applyTheme() { document.documentElement.setAttribute("data-theme", preferredTheme === "system" ? (scheme.matches ? "dark" : "light") : preferredTheme); }
scheme.addEventListener("change", applyTheme); applyTheme();
function render(s: Suggestion | null) {
  pending = s; accept.disabled = !s; dismiss.disabled = !s;
  const pair=document.getElementById("learning-pair")!;
  pair.textContent = s ? `Add ‘${s.candidate.correct}’ to dictionary?` : "";
  pair.title=pair.textContent;
  document.getElementById("learning-error")!.textContent = ""; tick();
}
function tick() {
  if (!pending) return;
  const remaining = pending.expires_at - Date.now();
  const duration = Math.max(1, pending.expires_at - pending.created_at);
  const elapsed = Math.max(0, Math.min(1, 1 - remaining / duration));
  const progress = document.getElementById("learning-progress")!;
  progress.setAttribute("aria-valuenow", String(Math.round(elapsed * 100)));
  progress.querySelector<HTMLElement>("span")!.style.transform = `scaleX(${elapsed})`;
  if (remaining <= 0) { const id = pending.id; render(null); void invoke("dismiss_learning_suggestion", { id }).catch(showError); }
}
setInterval(tick, 50);
accept.onclick = async () => {
  if (!pending) return; accept.disabled = true; dismiss.disabled = true;
  try { await invoke("accept_learning_suggestion", { id: pending.id }); render(null); }
  catch (err) { showError(err); accept.disabled = !pending; dismiss.disabled = !pending; }
};
function showError(err:unknown) {
  const error=document.getElementById("learning-error")!;
  error.textContent=pending ? "Could not save. Try Yes again." : "Add the term in Dictionary.";
  error.title=String(err);
}
dismiss.onclick = async () => { try { if (pending) await invoke("dismiss_learning_suggestion", { id: pending.id }); render(null); } catch(err) {showError(err);} };
document.onkeydown = e => { if (e.key === "Escape") dismiss.click(); };
async function start() {
  await listen<Suggestion | null>("dictionary-suggestion", event => render(event.payload));
  await listen<string>("learning-theme", event => { preferredTheme = event.payload; applyTheme(); });
  await listen<{theme:string}>("theme-changed", event => { preferredTheme = event.payload.theme; applyTheme(); });
  render(await invoke<Suggestion | null>("get_learning_suggestion"));
}
void start().catch(err => { document.getElementById("learning-error")!.textContent = String(err); });
