import { convertFileSrc } from "@tauri-apps/api/core";
import * as api from "$lib/tauri-bridge";

const LOCALMOD_GEN_MD = /!\[([^\]]*)\]\(localmod-gen:([^)]+)\)/g;
const LOCALMOD_GEN_BARE = /localmod-gen:([0-9a-f-]{36}\.png)/gi;
const SAVED_AS_FILE = /Saved as:\s*([0-9a-f-]{36}\.png)/gi;
const UUID_PNG =
  /\b([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.png)\b/gi;
const IMAGE_MD = /!\[([^\]]*)\]\(([^)]+)\)/g;

export type AssistantMarkdownBlock =
  | { kind: "text"; content: string }
  | { kind: "image"; alt: string; filename: string; src: string };

async function loadImageSrc(filename: string): Promise<string | null> {
  const file = filename.trim();
  if (!file) return null;
  try {
    return await api.readGeneratedImageDataUrl(file);
  } catch {
    return null;
  }
}

function isDisplayableImageSrc(src: string): boolean {
  return (
    src.startsWith("data:image/") ||
    src.startsWith("asset:") ||
    src.startsWith("http://") ||
    src.startsWith("https://") ||
    src.includes("localmod-gen:")
  );
}

function blocksFromResolved(
  resolved: string,
  filenameBySrc: Map<string, string>,
): AssistantMarkdownBlock[] {
  const blocks: AssistantMarkdownBlock[] = [];
  let last = 0;

  for (const m of resolved.matchAll(IMAGE_MD)) {
    const i = m.index ?? 0;
    if (i > last) {
      const text = resolved.slice(last, i).trim();
      if (text) blocks.push({ kind: "text", content: text });
    }
    const src = m[2]?.trim() ?? "";
    const alt = m[1]?.trim() || "Generated image";
    if (isDisplayableImageSrc(src)) {
      blocks.push({
        kind: "image",
        alt,
        filename: filenameBySrc.get(src) ?? "generated.png",
        src,
      });
    } else {
      blocks.push({ kind: "text", content: m[0] });
    }
    last = i + m[0].length;
  }

  const tail = resolved.slice(last).trim();
  if (tail) blocks.push({ kind: "text", content: tail });
  if (blocks.length === 0 && resolved.trim()) {
    blocks.push({ kind: "text", content: resolved.trim() });
  }
  return blocks;
}

async function replaceLocalmodGenRefs(
  md: string,
  filenameBySrc: Map<string, string>,
): Promise<string> {
  let out = md;
  const seen = new Set<string>();

  for (const m of md.matchAll(LOCALMOD_GEN_MD)) {
    const file = m[2]?.trim();
    if (!file || seen.has(file)) continue;
    seen.add(file);
    const src = await loadImageSrc(file);
    if (src) {
      filenameBySrc.set(src, file);
      out = out.replaceAll(m[0], `![${m[1] || "Generated image"}](${src})`);
    }
  }

  for (const m of out.matchAll(LOCALMOD_GEN_BARE)) {
    const file = m[1]?.trim();
    if (!file || seen.has(file)) continue;
    seen.add(file);
    const src = await loadImageSrc(file);
    if (src) {
      filenameBySrc.set(src, file);
      out = out.replaceAll(`localmod-gen:${file}`, src);
    }
  }

  return out;
}

async function appendMissingGeneratedImages(
  md: string,
  filenameBySrc: Map<string, string>,
): Promise<string> {
  const seen = new Set<string>();
  for (const m of md.matchAll(LOCALMOD_GEN_MD)) {
    if (m[2]) seen.add(m[2].trim());
  }
  for (const m of md.matchAll(LOCALMOD_GEN_BARE)) {
    if (m[1]) seen.add(m[1].trim());
  }

  const candidates = new Set<string>();
  for (const re of [SAVED_AS_FILE, UUID_PNG]) {
    for (const m of md.matchAll(re)) {
      const file = m[1]?.trim();
      if (file && !seen.has(file)) candidates.add(file);
    }
  }

  let out = md;
  for (const file of candidates) {
    const src = await loadImageSrc(file);
    if (src) {
      filenameBySrc.set(src, file);
      out = `${out.trim()}\n\n![Generated image](${src})`;
      seen.add(file);
    }
  }
  return out;
}

/** Split assistant markdown into text and generated-image blocks for chat UI. */
export async function buildAssistantMarkdownBlocks(
  md: string,
): Promise<AssistantMarkdownBlock[]> {
  const trimmed = md.trim();
  if (!trimmed) return [];

  const hasGenerated =
    trimmed.includes("localmod-gen:") ||
    trimmed.includes("data:image/") ||
    /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.png/i.test(
      trimmed,
    );

  if (!hasGenerated && !IMAGE_MD.test(trimmed)) {
    return [{ kind: "text", content: trimmed }];
  }
  IMAGE_MD.lastIndex = 0;

  const filenameBySrc = new Map<string, string>();
  let resolved = await replaceLocalmodGenRefs(trimmed, filenameBySrc);
  resolved = await appendMissingGeneratedImages(resolved, filenameBySrc);

  if (!IMAGE_MD.test(resolved)) {
    return resolved.trim() ? [{ kind: "text", content: resolved.trim() }] : [];
  }
  IMAGE_MD.lastIndex = 0;

  return blocksFromResolved(resolved, filenameBySrc);
}

/** Resolve `localmod-gen:` refs to displayable image URLs (data URLs via Rust). */
export async function resolveAssistantMarkdownImages(md: string): Promise<string> {
  const blocks = await buildAssistantMarkdownBlocks(md);
  return blocks
    .map((b) =>
      b.kind === "text"
        ? b.content
        : `![${b.alt}](${b.src})`,
    )
    .join("\n\n");
}

/** Sync fallback using Tauri asset URLs (requires asset protocol scope). */
export function resolveLocalmodGenImages(
  md: string,
  generatedDir: string,
): string {
  if (!md.includes("localmod-gen:") || !generatedDir) return md;
  const base = generatedDir.replace(/\\/g, "/").replace(/\/$/, "");
  return md.replace(
    /!\[([^\]]*)\]\(localmod-gen:([^)]+)\)/g,
    (_match, alt: string, file: string) => {
      const path = `${base}/${file}`;
      try {
        return `![${alt}](${convertFileSrc(path)})`;
      } catch {
        return `![${alt}](localmod-gen:${file})`;
      }
    },
  );
}
