/** Internal: Rust notifies the UI after the user row is committed, before assistant streaming. */
export const LM_STREAM_PHASE_USER_SAVED = "LMPHASE:user_saved";

/** Rust notifies the UI when a tool is running (e.g. web_search, read_file). */
export function parseLmToolPhase(raw: string): string | null {
  const prefix = "LMPHASE:tool:";
  if (raw.startsWith(prefix)) {
    return raw.slice(prefix.length) || "tool";
  }
  return null;
}

/** JSON lines emitted by the Rust sidecar for streaming chat. */
export type LmStreamChunk = { t: "r" | "c"; s: string };

export function parseLmStreamChunk(raw: string): LmStreamChunk | null {
  try {
    const m = JSON.parse(raw) as { t?: string; s?: string };
    if (m.t === "r" || m.t === "c") {
      return { t: m.t, s: typeof m.s === "string" ? m.s : "" };
    }
  } catch {
    /* not JSON — legacy plain token */
  }
  return null;
}

/** Strip common chain-of-thought wrappers from assistant markdown (visible reply only). */
export function stripAssistantThinkNoise(text: string): string {
  let s = text;
  const openThink = ["<", "think", ">"].join("");
  const closeThink = ["<", "/", "think", ">"].join("");
  const thinkRe = new RegExp(
    openThink + "[\\s\\S]*?" + closeThink,
    "gi",
  );
  const openThinking = ["<", "thinking", ">"].join("");
  const closeThinking = ["<", "/", "thinking", ">"].join("");
  const thinkingRe = new RegExp(
    openThinking + "[\\s\\S]*?" + closeThinking,
    "gi",
  );
  s = s.replace(thinkRe, "");
  s = s.replace(thinkingRe, "");
  s = s.replace(/\[Think\][\s\S]*?\[\/Think\]/gi, "");
  return s.replace(/\n{3,}/g, "\n\n").trim();
}
