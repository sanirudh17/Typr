export interface AiModelCatalog {
  models: { id: string }[];
  fetched_at: number | null;
  source: "live" | "cached" | "starter";
  warning: string | null;
}

// Used only before discovery succeeds. This is not an allowlist.
export const AI_MODEL_DEFAULT = "qwen/qwen3.8-27b";
export const STARTER_AI_MODELS = [AI_MODEL_DEFAULT, "openai/gpt-oss-120b", "openai/gpt-oss-20b"];

export function validAiModelId(id: string): boolean {
  return id.length <= 160 && /^[a-zA-Z0-9/_.:-]+$/.test(id);
}

export function replacementAiModel(selected: string, ids: string[]): string | undefined {
  const family = selected.startsWith("openai/gpt-oss-") ? "openai/gpt-oss-"
    : selected.startsWith("qwen/") ? "qwen/" : "";
  const rank = (id: string): number => family && id.startsWith(family) ? 0
    : id === AI_MODEL_DEFAULT ? 1 : id.startsWith("qwen/qwen3") ? 2
    : id === "openai/gpt-oss-120b" ? 3 : id === "openai/gpt-oss-20b" ? 4 : 5;
  return ids.filter(id => id !== selected).sort((a, b) => rank(a) - rank(b) || (a < b ? -1 : a > b ? 1 : 0))[0];
}
