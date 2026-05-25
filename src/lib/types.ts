export type ChatRole = "user" | "assistant" | "system";

export interface ChatMessage {
  id: string;
  role: ChatRole;
  content: string;
  createdAt: string;
}

export interface ChatThread {
  id: string;
  title: string;
  modelName: string;
  updatedAt: string;
  preview: string;
}

export interface ModelEntry {
  id: string;
  name: string;
  quant: string;
  sizeGb: number;
  ramGb: number;
  tags: string[];
  path?: string;
  isLocal: boolean;
}

export interface ContextDoc {
  id: string;
  name: string;
  kind: string;
  chunks: number;
  status: "ready" | "indexing" | "error";
}

export interface DownloadTask {
  id: string;
  name: string;
  progress: number;
  status: "queued" | "downloading" | "paused" | "complete" | "error";
  speedMbps?: number;
  sizeGb: number;
}

export interface DashboardStats {
  gpuName: string;
  gpuUsagePct: number;
  vramUsedGb: number;
  vramTotalGb: number;
  ramUsedGb: number;
  ramTotalGb: number;
  cpuUsagePct: number;
  loadedModelName: string | null;
  activeChats: number;
  modelsDirGb: number;
  appFootprintMb: number;
}
