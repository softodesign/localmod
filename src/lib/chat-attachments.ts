import * as pdfjs from "pdfjs-dist";
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

pdfjs.GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

const TEXT_EXT = new Set([
  "txt",
  "md",
  "markdown",
  "json",
  "csv",
  "log",
  "xml",
  "html",
  "htm",
  "css",
  "js",
  "ts",
  "rs",
]);

/** Files only — no images in the composer (AI generates images via tools). */
export const CHAT_ATTACHMENT_ACCEPT =
  ".pdf,.txt,.md,.markdown,.json,.csv,.log,.xml,.html,.htm,.css,.js,.ts,application/pdf,text/plain,text/markdown";

export type OutgoingPart = { type: "text"; text: string };

export type PendingAttachment = {
  kind: "document";
  name: string;
  docType: "pdf" | "text";
  extractedText: string;
};

export type JsonAttachmentMeta = { kind: string; name: string };

export type OutgoingUserPayload = {
  /** Stored in SQLite `messages.content` */
  storageJson: string;
  displayText: string;
  attachmentMetas: JsonAttachmentMeta[];
};

function extOf(name: string): string {
  const i = name.lastIndexOf(".");
  if (i < 0) return "";
  return name.slice(i + 1).toLowerCase();
}

export async function extractPdfText(file: File): Promise<string> {
  const buf = await file.arrayBuffer();
  const pdf = await pdfjs.getDocument({ data: buf }).promise;
  const out: string[] = [];
  for (let i = 1; i <= pdf.numPages; i++) {
    const page = await pdf.getPage(i);
    const tc = await page.getTextContent();
    const line = tc.items
      .map((it) => ("str" in it && typeof it.str === "string" ? it.str : ""))
      .join(" ");
    out.push(line);
  }
  return out.join("\n").replace(/\s+/g, " ").trim();
}

export function maxBytesForFile(file: File): number {
  const ext = extOf(file.name);
  if (ext === "pdf") return 25 * 1024 * 1024;
  return 8 * 1024 * 1024;
}

export async function fileToPendingAttachment(file: File): Promise<PendingAttachment> {
  const ext = extOf(file.name);
  const lim = maxBytesForFile(file);
  if (file.size > lim) {
    throw new Error(
      `“${file.name}” is too large (${Math.round(file.size / 1024)} KB). Max for this type is ${Math.round(lim / 1024)} KB.`,
    );
  }

  if (file.type.startsWith("image/")) {
    throw new Error(
      `“${file.name}” is an image. Attach PDF or text files only — ask the model to generate images instead.`,
    );
  }

  if (ext === "pdf" || file.type === "application/pdf") {
    const text = await extractPdfText(file);
    if (!text) {
      throw new Error(`No extractable text in “${file.name}”. It may be scanned pages only.`);
    }
    return { kind: "document", name: file.name, docType: "pdf", extractedText: text };
  }

  if (TEXT_EXT.has(ext) || file.type.startsWith("text/")) {
    const text = await file.text();
    return { kind: "document", name: file.name, docType: "text", extractedText: text };
  }

  try {
    const text = await file.text();
    if (text.length > 0 && !/^[\x00-\x08\x0b\x0c\x0e-\x1f]*$/.test(text.slice(0, 200))) {
      return { kind: "document", name: file.name, docType: "text", extractedText: text };
    }
  } catch {
    /* ignore */
  }

  throw new Error(
    `“${file.name}” is not a supported type. Try PDF, TXT, or Markdown.`,
  );
}

function docMetaKind(docType: "pdf" | "text"): string {
  return docType === "pdf" ? "pdf" : "file";
}

/**
 * Builds the JSON persisted for a user turn and parallel arrays for optimistic UI.
 */
export function buildUserMessagePayload(
  draftText: string,
  pending: PendingAttachment[],
): OutgoingUserPayload | null {
  const text = draftText.trim();

  const attachmentMetas: JsonAttachmentMeta[] = pending.map((a) => ({
    kind: docMetaKind(a.docType),
    name: a.name,
  }));

  const parts: OutgoingPart[] = [];
  if (text) parts.push({ type: "text", text });
  for (const a of pending) {
    parts.push({
      type: "text",
      text: `\n\n### ${a.name}\n\n${a.extractedText}`,
    });
  }
  if (parts.length === 0) return null;
  const storageJson = JSON.stringify({
    _lm: 1,
    displayText: text,
    attachments: attachmentMetas,
    parts,
  });
  return { storageJson, displayText: text, attachmentMetas };
}

export function parseUserMessageParts(raw: string): {
  plain: string;
  attachments: JsonAttachmentMeta[];
} {
  const t = raw.trimStart();
  if (!t.startsWith("{")) {
    return { plain: raw, attachments: [] };
  }
  try {
    const j = JSON.parse(raw) as {
      _lm?: number;
      displayText?: string;
      display?: string;
      attachments?: JsonAttachmentMeta[];
    };
    if (j._lm === 1) {
      const plain =
        (typeof j.displayText === "string" ? j.displayText : j.display) ?? "";
      const attachments = Array.isArray(j.attachments) ? j.attachments : [];
      return { plain, attachments };
    }
  } catch {
    /* ignore */
  }
  return { plain: raw, attachments: [] };
}

/** Plain preview for chat list / snippets */
export function userMessageDisplay(raw: string): string {
  const { plain, attachments } = parseUserMessageParts(raw);
  if (attachments.length) {
    const fileBits = attachments.map((a) => a.name).join(", ");
    if (plain.trim()) return `${plain} · ${fileBits}`;
    return fileBits;
  }
  return plain || raw;
}
