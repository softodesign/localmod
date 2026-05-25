import type {
  ChatMessage,
  ChatThread,
  ContextDoc,
  DashboardStats,
  DownloadTask,
  ModelEntry,
} from "$lib/types";

export const mockDashboard: DashboardStats = {
  gpuName: "NVIDIA GeForce RTX 4070 (mock)",
  gpuUsagePct: 0,
  vramUsedGb: 0,
  vramTotalGb: 12,
  ramUsedGb: 6.2,
  ramTotalGb: 32,
  cpuUsagePct: 8,
  loadedModelName: null,
  activeChats: 3,
  modelsDirGb: 18.4,
  appFootprintMb: 42,
};

export const mockThreads: ChatThread[] = [
  {
    id: "c1",
    title: "Rust lifetimes refresher",
    modelName: "Mistral 7B Instruct",
    updatedAt: "2026-05-17T10:12:00",
    preview: "Explain `&'a str` vs `String` in a table…",
  },
  {
    id: "c2",
    title: "Tauri IPC sketch",
    modelName: "Llama 3.1 8B",
    updatedAt: "2026-05-16T18:40:00",
    preview: "We can stream tokens with channels…",
  },
  {
    id: "c3",
    title: "SQLite schema for chats",
    modelName: "Mistral 7B Instruct",
    updatedAt: "2026-05-15T09:05:00",
    preview: "messages table needs chat_id FK…",
  },
];

export const mockMessagesByChat: Record<string, ChatMessage[]> = {
  c1: [
    {
      id: "m1",
      role: "user",
      content:
        "Give a **short** refresher on Rust lifetimes with one `code` sample.",
      createdAt: "2026-05-17T10:11:30",
    },
    {
      id: "m2",
      role: "assistant",
      content:
        "A **lifetime** tells the borrow checker how long references are valid.\n\n```rust\nfn longest<'a>(x: &'a str, y: &'a str) -> &'a str {\n    if x.len() >= y.len() { x } else { y }\n}\n```\n\n*(Mock response — replace with real inference.)*",
      createdAt: "2026-05-17T10:12:00",
    },
  ],
  c2: [
    {
      id: "m3",
      role: "user",
      content: "Outline how Tauri might stream tokens to the Svelte UI.",
      createdAt: "2026-05-16T18:38:00",
    },
    {
      id: "m4",
      role: "assistant",
      content:
        "1. Rust side runs the runtime in a task.\n2. Emit events on a channel.\n3. Forward chunks via `emit` or invoke streaming.\n\n_Mock — wire this to your engine later._",
      createdAt: "2026-05-16T18:40:00",
    },
  ],
  c3: [
    {
      id: "m5",
      role: "user",
      content: "What columns for `messages`?",
      createdAt: "2026-05-15T09:03:00",
    },
    {
      id: "m6",
      role: "assistant",
      content:
        "Suggest: `id`, `chat_id`, `role`, `content`, `created_at` — optionally `token_count` later.",
      createdAt: "2026-05-15T09:05:00",
    },
  ],
};

export const mockModelCatalog: ModelEntry[] = [
  {
    id: "mdl-1",
    name: "Mistral 7B Instruct",
    quant: "Q4_K_M",
    sizeGb: 4.1,
    ramGb: 8,
    tags: ["chat", "instruct"],
    isLocal: true,
    path: "models/mistral-7b-q4.gguf",
  },
  {
    id: "mdl-2",
    name: "Llama 3.1 8B Instruct",
    quant: "Q5_K_M",
    sizeGb: 5.7,
    ramGb: 10,
    tags: ["chat", "coding"],
    isLocal: true,
    path: "models/llama-3.1-8b-q5.gguf",
  },
  {
    id: "mdl-3",
    name: "Phi-3 Mini",
    quant: "Q4_0",
    sizeGb: 2.2,
    ramGb: 6,
    tags: ["fast", "coding"],
    isLocal: false,
  },
  {
    id: "mdl-4",
    name: "Qwen2.5 14B Instruct",
    quant: "Q4_K_M",
    sizeGb: 8.4,
    ramGb: 12,
    tags: ["chat", "long-context"],
    isLocal: false,
  },
];

export const mockContextDocs: ContextDoc[] = [
  {
    id: "doc-1",
    name: "ProductRequirements.md",
    kind: "markdown",
    chunks: 42,
    status: "ready",
  },
  {
    id: "doc-2",
    name: "notes-meeting.txt",
    kind: "text",
    chunks: 18,
    status: "ready",
  },
  {
    id: "doc-3",
    name: "legacy-api.pdf",
    kind: "pdf",
    chunks: 0,
    status: "indexing",
  },
];

export const mockDownloads: DownloadTask[] = [
  {
    id: "dl-1",
    name: "Qwen2.5-14B-Instruct-Q4_K_M.gguf",
    progress: 0.34,
    status: "downloading",
    speedMbps: 12.4,
    sizeGb: 8.4,
  },
  {
    id: "dl-2",
    name: "bge-small-en-v1.5.gguf",
    progress: 1,
    status: "complete",
    sizeGb: 0.13,
  },
  {
    id: "dl-3",
    name: "Phi-3-mini-Q4.gguf",
    progress: 0.6,
    status: "paused",
    sizeGb: 2.2,
  },
];

export const mockModelsForSwitcher = [
  "Mistral 7B Instruct",
  "Llama 3.1 8B Instruct",
  "Phi-3 Mini (not installed)",
];
