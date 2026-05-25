import { Channel, invoke } from "@tauri-apps/api/core";

export type MessageDto = {
  id: string;
  chatId: string;
  role: string;
  content: string;
  createdAt: string;
};

export type ChatDto = {
  id: string;
  title: string;
  modelId: string | null;
  modelName: string | null;
  createdAt: string;
  updatedAt: string;
  preview: string | null;
  /** Per-chat instructions sent as an extra leading `system` turn. */
  systemPrompt: string;
  projectId: string | null;
};

export type ProjectDto = {
  id: string;
  name: string;
  description: string;
  tags: string[];
  context: string;
  createdAt: string;
  updatedAt: string;
  chatCount: number;
};

export type ChatContextUsageDto = {
  usedTokens: number;
  limitTokens: number;
  remainingTokens: number;
  reservedOutputTokens: number;
  usedPercent: number;
};

export type ModelDto = {
  id: string;
  name: string;
  path: string;
  quant: string | null;
  sizeBytes: number | null;
  createdAt: string;
  weightsFormat: string;
  shardIndex: number | null;
  shardTotal: number | null;
  /** `chat` or `cloud` */
  modelKind: string;
};

export type LoadedDto = {
  id: string;
  name: string;
  path: string;
  /** Set when an mmproj GGUF is found next to the main model (auto-detected). */
  mmprojPath: string | null;
  modelKind: string;
};

export type LlmRuntimeStatusDto = {
  phase: "idle" | "loading" | "ready" | "generating" | "failed" | string;
  modelId: string | null;
  modelName: string | null;
  error: string | null;
  baseUrl: string | null;
};

export type BenchmarkPromptResultDto = {
  id: string;
  title: string;
  category: string;
  prompt: string;
  output: string;
  error: string | null;
  latencyMs: number;
  chars: number;
  estimatedTokens: number;
  tokensPerSecond: number;
};

export type BenchmarkModelResultDto = {
  modelId: string;
  modelName: string;
  weightsFormat: string;
  loadMs: number;
  totalMs: number;
  ramBeforeGb: number;
  ramAfterGb: number;
  ramDeltaGb: number;
  avgLatencyMs: number;
  avgTokensPerSecond: number;
  totalEstimatedTokens: number;
  prompts: BenchmarkPromptResultDto[];
};

export type BenchmarkRunDto = {
  runId: string;
  createdAt: string;
  models: BenchmarkModelResultDto[];
};

export type DashboardDto = {
  chatCount: number;
  modelCount: number;
  loaded: LoadedDto | null;
  modelsDirPath: string;
  modelsDirUsedBytes: number;
  appDataUsedBytes: number;
};

export type GpuMetrics = {
  name: string;
  usagePct: number | null;
  vramUsedMb: number | null;
  vramTotalMb: number | null;
};

export type DiskMountMetrics = {
  name: string;
  mount: string;
  totalGb: number;
  freeGb: number;
};

export type SystemSnapshot = {
  hostName: string | null;
  osVersion: string | null;
  cpuName: string;
  cpuCores: number;
  cpuUsagePct: number;
  ramUsedGb: number;
  ramTotalGb: number;
  swapUsedGb: number;
  swapTotalGb: number;
  gpus: GpuMetrics[];
  disks: DiskMountMetrics[];
};

export type PathsDto = {
  appDataDir: string;
  databasePath: string;
  modelsDir: string;
  contextDirResolved: string;
  builtinAppDataDir: string;
  configuredAppDataDir: string;
};

export type ApiServerSettingsDto = {
  enabled: boolean;
  host: string;
  port: number;
  authMode: "none" | "bearer" | string;
  apiKey: string;
};

export type ApiServerStatusDto = {
  running: boolean;
  host: string;
  port: number;
  authMode: "none" | "bearer" | string;
  baseUrl: string;
};

export type HeadlessServerStatusDto = {
  running: boolean;
  pid: number | null;
  host: string;
  port: number;
  authMode: "none" | "bearer" | string;
  baseUrl: string;
  dataDir: string;
  modelsDir: string;
  runtimeDir: string;
  command: string;
  lastError: string | null;
};

export type ContextDocDto = {
  id: string;
  name: string;
  source: string;
  kind: string;
  storedPath: string;
  sizeBytes: number | null;
  chunks: number;
  status: string;
  createdAt: string;
};

export async function getSettings(): Promise<[string, string][]> {
  return invoke("get_settings");
}

export async function setSetting(key: string, value: string): Promise<void> {
  return invoke("set_setting", { key, value });
}

export async function getApiServerSettings(): Promise<ApiServerSettingsDto> {
  return invoke<ApiServerSettingsDto>("get_api_server_settings");
}

export async function getApiServerStatus(): Promise<ApiServerStatusDto> {
  return invoke<ApiServerStatusDto>("get_api_server_status");
}

export async function getLlmRuntimeStatus(): Promise<LlmRuntimeStatusDto> {
  return invoke<LlmRuntimeStatusDto>("get_llm_runtime_status");
}

export async function validateLlamaRuntime(): Promise<string> {
  return invoke<string>("validate_llama_runtime");
}

export async function startApiServer(args: {
  enabled: boolean;
  host: string;
  port: number;
  authMode: string;
  apiKey: string;
}): Promise<ApiServerStatusDto> {
  return invoke<ApiServerStatusDto>("start_api_server", args);
}

export async function stopApiServer(): Promise<ApiServerStatusDto> {
  return invoke<ApiServerStatusDto>("stop_api_server");
}

export async function getHeadlessServerStatus(): Promise<HeadlessServerStatusDto> {
  return invoke<HeadlessServerStatusDto>("get_headless_server_status");
}

export async function startHeadlessServer(args: {
  host: string;
  port: number;
  dataDir: string;
  modelsDir: string;
  runtimeDir: string;
  authMode: string;
  apiKey: string;
}): Promise<HeadlessServerStatusDto> {
  return invoke<HeadlessServerStatusDto>("start_headless_server", { args });
}

export async function stopHeadlessServer(): Promise<HeadlessServerStatusDto> {
  return invoke<HeadlessServerStatusDto>("stop_headless_server");
}

export async function getDashboard(): Promise<DashboardDto> {
  return invoke<DashboardDto>("get_dashboard");
}

export async function getSystemSnapshot(): Promise<SystemSnapshot> {
  return invoke<SystemSnapshot>("get_system_snapshot");
}

export async function getPaths(): Promise<PathsDto> {
  return invoke<PathsDto>("get_paths");
}

export async function listModels(): Promise<ModelDto[]> {
  return invoke("list_models");
}

export async function registerModel(path: string): Promise<ModelDto> {
  return invoke("register_model", { path });
}

export async function updateModel(
  modelId: string,
  patch: {
    name?: string;
    path?: string;
  },
): Promise<ModelDto> {
  return invoke("update_model", {
    modelId,
    name: patch.name ?? null,
    path: patch.path ?? null,
  });
}

export async function runModelBenchmark(
  modelIds: string[],
): Promise<BenchmarkRunDto> {
  return invoke("run_model_benchmark", { modelIds });
}

export type HfWeightFileDto = {
  path: string;
  size: number | null;
  kind: "gguf" | "safetensors";
};

export async function listHuggingfaceGgufFiles(
  repoInput: string,
  revision: string | null,
): Promise<HfWeightFileDto[]> {
  return invoke("list_huggingface_gguf_files", { repoInput, revision });
}

export type HfDownloadJobDto = {
  id: string;
  title: string;
  status:
    | "queued"
    | "running"
    | "paused"
    | "completed"
    | "cancelled"
    | "failed";
  message: string;
  progress: number;
  bytesDownloaded: number;
  bytesTotal: number | null;
  currentFile: string | null;
  fileIndex: number;
  fileCount: number;
  registeredModelId: string | null;
  error: string | null;
};

export async function hfDownloadList(): Promise<HfDownloadJobDto[]> {
  return invoke<HfDownloadJobDto[]>("hf_download_list");
}

export async function hfDownloadStartAuto(repoInput: string): Promise<string> {
  return invoke("hf_download_start_auto", { repoInput });
}

export async function hfDownloadStartManual(
  repoInput: string,
  filePath: string,
  revision: string | null,
): Promise<string> {
  return invoke("hf_download_start_manual", {
    repoInput,
    filePath,
    revision,
  });
}

export async function hfDownloadPause(jobId: string): Promise<void> {
  return invoke("hf_download_pause", { jobId });
}

export async function hfDownloadResume(jobId: string): Promise<void> {
  return invoke("hf_download_resume", { jobId });
}

export async function hfDownloadCancel(jobId: string): Promise<void> {
  return invoke("hf_download_cancel", { jobId });
}

export async function hfDownloadDismiss(jobId: string): Promise<void> {
  return invoke("hf_download_dismiss", { jobId });
}

export type ModelKnowledgeDto = {
  displayName: string;
  supportsImages: boolean;
  visionExplanation: string;
  summary: string;
  capabilities: string[];
};

export async function getModelKnowledge(
  path: string,
): Promise<ModelKnowledgeDto> {
  return invoke("get_model_knowledge", { path });
}

export async function deleteModel(id: string): Promise<void> {
  return invoke("delete_model", { id });
}

export type CloudProviderUiDto = {
  id: string;
  model: string;
  hasApiKey: boolean;
  baseUrl?: string;
  imageGenerationEnabled?: boolean;
  imageModel?: string;
};

export async function getCloudProviderConfigs(): Promise<CloudProviderUiDto[]> {
  return invoke("get_cloud_provider_configs");
}

export async function setCloudProviderConfig(
  provider: string,
  apiKey: string,
  model: string,
  baseUrl?: string | null,
  imageGenerationEnabled?: boolean,
  imageModel?: string | null,
): Promise<void> {
  return invoke("set_cloud_provider_config", {
    provider,
    apiKey,
    model,
    baseUrl: baseUrl ?? null,
    imageGenerationEnabled: imageGenerationEnabled ?? null,
    imageModel: imageModel ?? null,
  });
}

export async function chatImageGenAvailable(
  chatId: string,
  modelId?: string | null,
): Promise<boolean> {
  return invoke("chat_image_gen_available", {
    chatId,
    modelId: modelId ?? null,
  });
}

export async function readGeneratedImageDataUrl(
  filename: string,
): Promise<string> {
  return invoke("read_generated_image_data_url", { filename });
}

export async function exportGeneratedImage(
  filename: string,
  destPath: string,
): Promise<void> {
  return invoke("export_generated_image", { filename, destPath });
}

export async function listChats(): Promise<ChatDto[]> {
  return invoke("list_chats");
}

export async function createChat(
  title: string | null,
  modelId: string | null,
  projectId?: string | null,
): Promise<ChatDto> {
  return invoke("create_chat", { title, modelId, projectId: projectId ?? null });
}

export async function listProjects(): Promise<ProjectDto[]> {
  return invoke("list_projects");
}

export async function createProject(
  name: string,
  description?: string | null,
  tags?: string[] | null,
): Promise<ProjectDto> {
  return invoke("create_project", {
    name,
    description: description ?? null,
    tags: tags ?? null,
  });
}

export async function updateProject(
  projectId: string,
  patch: {
    name?: string;
    description?: string;
    tags?: string[];
    context?: string;
  },
): Promise<ProjectDto> {
  return invoke("update_project", {
    projectId,
    name: patch.name ?? null,
    description: patch.description ?? null,
    tags: patch.tags ?? null,
    context: patch.context ?? null,
  });
}

export async function deleteProject(projectId: string): Promise<void> {
  return invoke("delete_project", { projectId });
}

export async function getChatContextUsage(
  chatId: string,
  draft?: string | null,
  contextDocumentIds?: string[] | null,
): Promise<ChatContextUsageDto> {
  return invoke("get_chat_context_usage", {
    chatId,
    draft: draft ?? null,
    contextDocumentIds: contextDocumentIds ?? null,
  });
}

export async function renameChat(chatId: string, title: string): Promise<void> {
  return invoke("rename_chat", { chatId, title });
}

export async function setChatModel(
  chatId: string,
  modelId: string,
): Promise<void> {
  return invoke("set_chat_model", { chatId, modelId });
}

export async function setChatSystemPrompt(
  chatId: string,
  systemPrompt: string,
): Promise<void> {
  return invoke("set_chat_system_prompt", { chatId, systemPrompt });
}

export async function deleteChat(id: string): Promise<void> {
  return invoke("delete_chat", { id });
}

export async function listMessages(chatId: string): Promise<MessageDto[]> {
  return invoke("list_messages", { chatId });
}

export async function listContextDocuments(): Promise<ContextDocDto[]> {
  return invoke("list_context_documents");
}

export async function addContextFromPath(path: string): Promise<ContextDocDto> {
  return invoke("add_context_from_path", { path });
}

export async function addContextText(
  title: string,
  description: string,
  content: string,
): Promise<ContextDocDto> {
  return invoke("add_context_text", { title, description, content });
}

export type ContextTextEditDto = {
  id: string;
  name: string;
  description: string;
  content: string;
};

export async function getContextTextForEdit(
  id: string,
): Promise<ContextTextEditDto> {
  return invoke("get_context_text_for_edit", { id });
}

export async function updateContextText(
  id: string,
  title: string,
  description: string,
  content: string,
): Promise<ContextDocDto> {
  return invoke("update_context_text", { id, title, description, content });
}

export async function deleteContextDocument(id: string): Promise<void> {
  return invoke("delete_context_document", { id });
}

export async function loadLlm(modelId: string): Promise<LoadedDto> {
  return invoke("load_llm", { modelId });
}

export async function unloadLlm(): Promise<void> {
  return invoke("unload_llm");
}

export async function getLoadedLlm(): Promise<LoadedDto | null> {
  return invoke("get_loaded_llm");
}

export async function stopGeneration(): Promise<void> {
  return invoke("stop_generation");
}

export type SendResult = {
  assistantMessageId: string;
  cancelled: boolean;
};

export async function sendChatMessageStream(
  chatId: string,
  content: string,
  thinkingEnabled: boolean,
  webSearchEnabled: boolean,
  agentEnabled: boolean,
  imageGenerationEnabled: boolean,
  onToken: (chunk: string) => void,
  contextDocumentIds: string[] = [],
  headerModelId?: string | null,
): Promise<SendResult> {
  const ch = new Channel<string>();
  ch.onmessage = onToken;
  return invoke("send_chat_message", {
    onToken: ch,
    chatId,
    content,
    thinkingEnabled,
    webSearchEnabled,
    agentEnabled,
    imageGenerationEnabled,
    contextDocumentIds,
    headerModelId: headerModelId ?? null,
  });
}

export async function regenerateAssistantMessageStream(
  chatId: string,
  assistantMessageId: string,
  thinkingEnabled: boolean,
  webSearchEnabled: boolean,
  agentEnabled: boolean,
  imageGenerationEnabled: boolean,
  onToken: (chunk: string) => void,
  headerModelId?: string | null,
): Promise<SendResult> {
  const ch = new Channel<string>();
  ch.onmessage = onToken;
  return invoke("regenerate_assistant_message", {
    onToken: ch,
    chatId,
    assistantMessageId,
    thinkingEnabled,
    webSearchEnabled,
    agentEnabled,
    imageGenerationEnabled,
    headerModelId: headerModelId ?? null,
  });
}

export async function deleteMessage(messageId: string): Promise<void> {
  return invoke("delete_message", { messageId });
}

export async function updateMessage(
  messageId: string,
  content: string,
): Promise<void> {
  return invoke("update_message", { messageId, content });
}
