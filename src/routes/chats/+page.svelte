<script lang="ts">
  import { onMount, tick } from "svelte";
  import { afterNavigate } from "$app/navigation";
  import AssistantMarkdown from "$lib/components/AssistantMarkdown.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import * as api from "$lib/tauri-bridge";
  import {
    isLoadableModel,
    loadableModels,
    resolveLoadableModelId,
  } from "$lib/model-utils";
  import {
    CHAT_ATTACHMENT_ACCEPT,
    fileToPendingAttachment,
    userMessageDisplay,
    buildUserMessagePayload,
    parseUserMessageParts,
    type OutgoingUserPayload,
    type PendingAttachment,
  } from "$lib/chat-attachments";
  import Plus from "@lucide/svelte/icons/plus";
  import FolderPlus from "@lucide/svelte/icons/folder-plus";
  import Settings from "@lucide/svelte/icons/settings";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import Paperclip from "@lucide/svelte/icons/paperclip";
  import FileText from "@lucide/svelte/icons/file-text";
  import File from "@lucide/svelte/icons/file";
  import ImagePlus from "@lucide/svelte/icons/image-plus";
  import Brain from "@lucide/svelte/icons/brain";
  import Wrench from "@lucide/svelte/icons/wrench";
  import Globe from "@lucide/svelte/icons/globe";
  import Terminal from "@lucide/svelte/icons/terminal";
  import PanelLeft from "@lucide/svelte/icons/panel-left";
  import PanelLeftClose from "@lucide/svelte/icons/panel-left-close";
  import NotepadText from "@lucide/svelte/icons/notepad-text";
  import Square from "@lucide/svelte/icons/square";
  import Clipboard from "@lucide/svelte/icons/clipboard";
  import Check from "@lucide/svelte/icons/check";
  import {
    LM_STREAM_PHASE_USER_SAVED,
    parseLmStreamChunk,
    parseLmToolPhase,
    stripAssistantThinkNoise,
  } from "$lib/chat-assistant";
  import ContextRing from "$lib/components/ui/ContextRing.svelte";

  const THINKING_LS_KEY = "localmod.chat.thinkingEnabled";
  const WEB_SEARCH_LS_KEY = "localmod.chat.webSearchEnabled";
  const AGENT_LS_KEY = "localmod.chat.agentEnabled";
  const IMAGE_GEN_LS_KEY = "localmod.chat.imageGenEnabled";
  const SIDEBAR_W_LS_KEY = "localmod.chat.sidebarWidth";
  const SIDEBAR_COLLAPSED_LS_KEY = "localmod.chat.sidebarCollapsed";
  const EXPANDED_PROJECTS_LS_KEY = "localmod.chat.expandedProjects";
  const SIDEBAR_W_MIN = 200;
  const SIDEBAR_W_MAX = 480;
  const SIDEBAR_W_DEFAULT = 280;

  function clampSidebarWidth(w: number) {
    return Math.min(SIDEBAR_W_MAX, Math.max(SIDEBAR_W_MIN, w));
  }

  function persistSidebarPrefs() {
    try {
      localStorage.setItem(SIDEBAR_W_LS_KEY, String(sidebarWidth));
      localStorage.setItem(
        SIDEBAR_COLLAPSED_LS_KEY,
        sidebarCollapsed ? "1" : "0",
      );
    } catch {
      /* ignore */
    }
  }

  function toggleSidebarCollapsed() {
    sidebarCollapsed = !sidebarCollapsed;
    persistSidebarPrefs();
  }

  function onSidebarResizeStart(e: MouseEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startW = sidebarWidth;
    sidebarResizing = true;
    try {
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
    } catch {
      /* ignore */
    }
    const onMove = (ev: MouseEvent) => {
      sidebarWidth = clampSidebarWidth(startW + (ev.clientX - startX));
    };
    const onUp = () => {
      sidebarResizing = false;
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      try {
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
      } catch {
        /* ignore */
      }
      persistSidebarPrefs();
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  }

  let chats = $state<api.ChatDto[]>([]);
  let projects = $state<api.ProjectDto[]>([]);
  let expandedProjectIds = $state<Set<string>>(new Set());
  let messages = $state<api.MessageDto[]>([]);
  let models = $state<api.ModelDto[]>([]);
  let loaded = $state<api.LoadedDto | null>(null);
  let runtimeStatus = $state<api.LlmRuntimeStatusDto | null>(null);
  let selectedId = $state("");
  let draft = $state("");
  let loadModelId = $state("");
  let sending = $state(false);
  let streamText = $state("");
  let err = $state<string | null>(null);
  let messagesLoading = $state(false);
  let pendingSelectedId = $state<string | null>(null);

  /** When true, llama-server gets `chat_template_kwargs.enable_thinking` (Qwen-style). */
  let thinkingEnabled = $state(true);
  /** When true, the model can call web_search during the turn. */
  let webSearchEnabled = $state(false);
  /** When true, the model can use file/command/system agent tools. */
  let agentEnabled = $state(false);
  /** When true, the model can call generate_image during the turn. */
  let imageGenEnabled = $state(false);
  let imageGenAvailable = $state(false);
  let toolsMenuOpen = $state(false);
  /** True while the model streams reasoning deltas (hidden from the bubble; UI shows “Thinking…”). */
  let streamInReasoning = $state(false);
  /** Label for an in-flight tool (web_search, read_file, …). */
  let streamToolLabel = $state<string | null>(null);
  let runtimeStatusTimer: ReturnType<typeof setInterval> | undefined;

  let renamingId = $state<string | null>(null);
  let renameDraft = $state("");

  let pendingAttachments = $state<PendingAttachment[]>([]);
  let fileInputEl: HTMLInputElement | undefined = $state();

  let optimisticUser = $state<OutgoingUserPayload | null>(null);

  let editModalOpen = $state(false);
  let editMessageId = $state("");
  let editMessageRole = $state<"user" | "assistant">("user");
  let editMessageRaw = $state("");
  let editDraft = $state("");
  let editSaveBusy = $state(false);

  /** Context library docs for @mentions (same as Settings → Context). */
  let contextDocs = $state<api.ContextDocDto[]>([]);
  let textareaRef: HTMLTextAreaElement | undefined = $state();
  let mentionOpen = $state(false);
  let mentionStart = $state(-1);
  let mentionQuery = $state("");
  let mentionPickIndex = $state(0);

  let sidebarWidth = $state(SIDEBAR_W_DEFAULT);
  let sidebarCollapsed = $state(false);
  let sidebarResizing = $state(false);

  let systemPromptOpen = $state(false);
  let systemPromptDraft = $state("");

  let createProjectOpen = $state(false);
  let createProjectName = $state("");
  let createProjectDesc = $state("");
  let createProjectTags = $state("");
  let createProjectBusy = $state(false);

  let projectSettingsOpen = $state(false);
  let projectSettingsBusy = $state(false);
  let projectSettingsId = $state("");
  let projectSettingsName = $state("");
  let projectSettingsDesc = $state("");
  let projectSettingsTags = $state("");
  let projectSettingsContext = $state("");

  let contextUsage = $state<api.ChatContextUsageDto | null>(null);
  let contextUsageTimer: ReturnType<typeof setTimeout> | undefined;

  /** Invalidates in-flight `listMessages` results when switching threads quickly. */
  let loadMessagesSeq = 0;

  const ungroupedChats = $derived(chats.filter((c) => !c.projectId));

  function chatsForProject(projectId: string) {
    return chats.filter((c) => c.projectId === projectId);
  }

  function isProjectExpanded(projectId: string) {
    return expandedProjectIds.has(projectId);
  }

  function toggleProjectExpanded(projectId: string) {
    const next = new Set(expandedProjectIds);
    if (next.has(projectId)) next.delete(projectId);
    else next.add(projectId);
    expandedProjectIds = next;
    try {
      localStorage.setItem(
        EXPANDED_PROJECTS_LS_KEY,
        JSON.stringify([...next]),
      );
    } catch {
      /* ignore */
    }
  }

  function parseTagsInput(raw: string): string[] {
    return raw
      .split(/[,;]+/)
      .map((t) => t.trim())
      .filter(Boolean);
  }

  function formatTags(tags: string[]): string {
    return tags.join(", ");
  }

  async function refreshProjects() {
    try {
      projects = await api.listProjects();
    } catch {
      projects = [];
    }
  }

  function openCreateProjectModal() {
    if (uiLockedForGeneration) return;
    createProjectName = "";
    createProjectDesc = "";
    createProjectTags = "";
    createProjectOpen = true;
  }

  function closeCreateProjectModal() {
    createProjectOpen = false;
  }

  async function saveCreateProject() {
    const name = createProjectName.trim();
    if (!name) return;
    createProjectBusy = true;
    err = null;
    try {
      const p = await api.createProject(
        name,
        createProjectDesc.trim() || null,
        parseTagsInput(createProjectTags),
      );
      const next = new Set(expandedProjectIds);
      next.add(p.id);
      expandedProjectIds = next;
      try {
        localStorage.setItem(
          EXPANDED_PROJECTS_LS_KEY,
          JSON.stringify([...next]),
        );
      } catch {
        /* ignore */
      }
      await refreshProjects();
      closeCreateProjectModal();
    } catch (e) {
      err = String(e);
    } finally {
      createProjectBusy = false;
    }
  }

  function openProjectSettings(p: api.ProjectDto) {
    if (uiLockedForGeneration) return;
    projectSettingsId = p.id;
    projectSettingsName = p.name;
    projectSettingsDesc = p.description;
    projectSettingsTags = formatTags(p.tags);
    projectSettingsContext = p.context;
    projectSettingsOpen = true;
  }

  function closeProjectSettings() {
    projectSettingsOpen = false;
  }

  async function saveProjectSettings() {
    if (!projectSettingsId) return;
    projectSettingsBusy = true;
    err = null;
    try {
      await api.updateProject(projectSettingsId, {
        name: projectSettingsName.trim(),
        description: projectSettingsDesc,
        tags: parseTagsInput(projectSettingsTags),
        context: projectSettingsContext,
      });
      await refreshProjects();
      closeProjectSettings();
      scheduleContextUsageRefresh();
    } catch (e) {
      err = String(e);
    } finally {
      projectSettingsBusy = false;
    }
  }

  async function removeProject(p: api.ProjectDto) {
    if (uiLockedForGeneration) return;
    if (
      !confirm(
        `Delete project "${p.name}"? Chats in this project will move to the general list.`,
      )
    ) {
      return;
    }
    err = null;
    try {
      await api.deleteProject(p.id);
      await Promise.all([refreshProjects(), refreshChatList()]);
      scheduleContextUsageRefresh();
    } catch (e) {
      err = String(e);
    }
  }

  function scheduleContextUsageRefresh() {
    if (contextUsageTimer !== undefined) clearTimeout(contextUsageTimer);
    contextUsageTimer = setTimeout(() => {
      contextUsageTimer = undefined;
      void refreshContextUsage();
    }, 180);
  }

  async function refreshContextUsage() {
    if (!selectedId) {
      contextUsage = null;
      return;
    }
    try {
      const ids = resolveContextIdsFromDisplay(draft);
      contextUsage = await api.getChatContextUsage(
        selectedId,
        draft.trim() || null,
        ids.length ? ids : null,
      );
    } catch {
      contextUsage = null;
    }
  }

  const contextRingDetails = $derived.by(() => {
    if (!contextUsage) return "";
    return [
      `Used: ~${contextUsage.usedTokens.toLocaleString()} tokens`,
      `Limit: ${contextUsage.limitTokens.toLocaleString()} tokens`,
      `Remaining: ~${contextUsage.remainingTokens.toLocaleString()} tokens`,
      `Reserved for reply: ${contextUsage.reservedOutputTokens.toLocaleString()} tokens`,
    ].join("\n");
  });

  const loadableModelsList = $derived(loadableModels(models));
  const activeImageGenEnabled = $derived(imageGenEnabled && imageGenAvailable);
  const imageGenToolsHint = $derived.by(() => {
    if (imageGenAvailable) {
      return "Uses your cloud provider image model (Models → Cloud)";
    }
    return "Enable cloud image generation under Models → Cloud providers";
  });

  $effect(() => {
    void selectedId;
    void loadModelId;
    void refreshImageGenAvailable();
  });

  function effectiveModelId(): string {
    return resolveLoadableModelId(models, [
      loadModelId,
      chats.find((c) => c.id === selectedId)?.modelId,
      loaded?.id,
    ]);
  }

  async function refreshImageGenAvailable() {
    if (!selectedId) {
      imageGenAvailable = false;
      return;
    }
    try {
      const mid = effectiveModelId();
      imageGenAvailable = await api.chatImageGenAvailable(selectedId, mid || null);
    } catch {
      imageGenAvailable = false;
    }
  }

  afterNavigate(({ to }) => {
    if (to?.url.pathname === "/chats" || to?.url.pathname.startsWith("/chats/")) {
      void refreshImageGenAvailable();
    }
  });

  /** Persist header model to this chat; auto-load cloud models (no GGUF reload). */
  async function onHeaderModelChange() {
    if (!selectedId || !loadModelId || sending) return;
    const m = models.find((x) => x.id === loadModelId);
    if (m && !isLoadableModel(m)) return;
    try {
      await api.setChatModel(selectedId, loadModelId);
      if (m?.weightsFormat === "cloud") {
        loaded = await api.loadLlm(loadModelId);
      }
      await refreshChatList();
    } catch (e) {
      err = String(e);
    }
    void refreshImageGenAvailable();
  }

  /** Header selection can differ from chats.model_id until Load / dropdown sync — fix before inference. */
  async function ensureChatModelSynced(): Promise<boolean> {
    if (!selectedId) return false;
    const mid = effectiveModelId();
    if (!mid) return false;
    const chat = chats.find((c) => c.id === selectedId);
    if (chat?.modelId === mid) return true;
    try {
      await api.setChatModel(selectedId, mid);
      await refreshChatList();
      return true;
    } catch (e) {
      err = String(e);
      return false;
    }
  }

  const mentionCandidates = $derived.by(() => {
    const q = mentionQuery.trim().toLowerCase();
    const list = !q
      ? contextDocs
      : contextDocs.filter((d) => d.name.toLowerCase().includes(q));
    return list.slice(0, 12);
  });

  /** Sidebar only — avoids double-fetching messages when used with `refreshMessages` in parallel. */
  async function refreshChatList() {
    chats = await api.listChats();
    await refreshProjects();
    if (selectedId && !chats.some((c) => c.id === selectedId)) {
      selectedId = chats[0]?.id ?? "";
    }
    if (!selectedId && chats[0]) {
      selectedId = chats[0].id;
    }
    scheduleContextUsageRefresh();
  }

  async function refreshChats() {
    await refreshChatList();
    await refreshMessages();
  }

  function startRename(t: api.ChatDto) {
    if (sending) return;
    renamingId = t.id;
    renameDraft = t.title;
  }

  function cancelRename() {
    renamingId = null;
    renameDraft = "";
  }

  async function commitRename() {
    if (!renamingId || sending) return;
    const t = renameDraft.trim();
    if (!t) return;
    err = null;
    try {
      await api.renameChat(renamingId, t);
      cancelRename();
      await Promise.all([refreshMessages(), refreshChatList()]);
    } catch (e) {
      err = String(e);
    }
  }

  function openSystemPromptModal() {
    if (!selectedId) return;
    const c = chats.find((x) => x.id === selectedId);
    systemPromptDraft = c?.systemPrompt ?? "";
    systemPromptOpen = true;
  }

  function closeSystemPromptModal() {
    systemPromptOpen = false;
  }

  async function saveSystemPrompt() {
    if (!selectedId) return;
    err = null;
    try {
      await api.setChatSystemPrompt(selectedId, systemPromptDraft);
      systemPromptOpen = false;
      await refreshChatList();
      scheduleContextUsageRefresh();
    } catch (e) {
      err = String(e);
    }
  }

  async function removeChat(t: api.ChatDto) {
    if (sending) return;
    if (!confirm(`Delete "${t.title}" and all messages?`)) return;
    err = null;
    try {
      await api.deleteChat(t.id);
      const wasSelected = selectedId === t.id;
      if (wasSelected) {
        selectedId = "";
        loadModelSeq += 1;
        loadModelInFlight = 0;
        loadModelBusy = false;
      }
      cancelRename();
      await refreshChatList();
      await refreshMessages();
      await refreshModelsAndLoaded();
    } catch (e) {
      err = String(e);
    }
  }

  async function refreshContextDocs() {
    try {
      contextDocs = await api.listContextDocuments();
    } catch {
      /* ignore */
    }
  }

  function mentionStateAtCaret(text: string, caret: number) {
    const before = text.slice(0, caret);
    const at = before.lastIndexOf("@");
    if (at < 0) return { open: false as const, query: "", start: -1 };
    const frag = before.slice(at + 1);
    if (/[\s\n]/.test(frag)) return { open: false as const, query: "", start: -1 };
    return { open: true as const, query: frag, start: at };
  }

  function updateMentionUi() {
    const el = textareaRef;
    if (!el) return;
    const st = mentionStateAtCaret(draft, el.selectionStart);
    if (st.open) {
      mentionOpen = true;
      mentionStart = st.start;
      mentionQuery = st.query;
      mentionPickIndex = 0;
      if (contextDocs.length === 0) void refreshContextDocs();
    } else {
      mentionOpen = false;
      mentionStart = -1;
      mentionQuery = "";
    }
  }

  /** Match @filename using full Context names (incl. spaces). Longest name wins first. */
  function resolveContextIdsFromDisplay(text: string): string[] {
    const ids: string[] = [];
    const seen = new Set<string>();
    if (!text || contextDocs.length === 0) return ids;

    const docs = [...contextDocs].sort((a, b) => b.name.length - a.name.length);

    for (let i = 0; i < text.length; ) {
      const at = text.indexOf("@", i);
      if (at < 0) break;
      const rest = text.slice(at + 1);

      let found: api.ContextDocDto | undefined;
      for (const d of docs) {
        const n = d.name;
        if (rest.length < n.length) continue;
        const chunk = rest.slice(0, n.length);
        if (chunk !== n && chunk.toLowerCase() !== n.toLowerCase()) continue;
        const boundary = rest[n.length];
        const boundaryOk =
          boundary === undefined ||
          /\s/.test(boundary) ||
          /[.,;:!?)\]}>«»'"，。]/.test(boundary);
        if (!boundaryOk) continue;
        found = d;
        break;
      }

      if (found && !seen.has(found.id)) {
        seen.add(found.id);
        ids.push(found.id);
      }
      i = at + 1;
    }
    return ids;
  }

  async function pickMention(doc: api.ContextDocDto) {
    const el = textareaRef;
    if (!el || mentionStart < 0) return;
    const caret = el.selectionStart;
    const before = draft.slice(0, mentionStart);
    const after = draft.slice(caret);
    draft = `${before}@${doc.name} ${after}`;
    mentionOpen = false;
    mentionStart = -1;
    await tick();
    const pos = before.length + doc.name.length + 2;
    el.setSelectionRange(pos, pos);
    el.focus();
  }

  function onDraftKeydown(e: KeyboardEvent) {
    if (!mentionOpen || mentionCandidates.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      mentionPickIndex = (mentionPickIndex + 1) % mentionCandidates.length;
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      mentionPickIndex =
        (mentionPickIndex - 1 + mentionCandidates.length) %
        mentionCandidates.length;
    } else if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      const d = mentionCandidates[mentionPickIndex];
      if (d) void pickMention(d);
    } else if (e.key === "Escape") {
      mentionOpen = false;
    }
  }

  async function refreshMessages() {
    const chatId = selectedId;
    if (!chatId) {
      loadMessagesSeq += 1;
      messages = [];
      messagesLoading = false;
      pendingSelectedId = null;
      return;
    }
    const ticket = ++loadMessagesSeq;
    messagesLoading = true;
    pendingSelectedId = chatId;
    try {
      const list = await api.listMessages(chatId);
      if (ticket !== loadMessagesSeq || selectedId !== chatId) {
        return;
      }
      messages = list;
      pendingSelectedId = null;
      scheduleContextUsageRefresh();
    } catch (e) {
      if (ticket === loadMessagesSeq && selectedId === chatId) {
        err = String(e);
      }
    } finally {
      if (ticket === loadMessagesSeq && selectedId === chatId) {
        messagesLoading = false;
        pendingSelectedId = null;
      }
    }
  }

  function resetThreadUi() {
    closeEditModal();
    copyFlashKey = null;
    clearTimeout(copyFlashTimer);
    copyFlashTimer = undefined;
    optimisticUser = null;
    streamText = "";
    streamInReasoning = false;
    streamToolLabel = null;
    toolsMenuOpen = false;
    mentionOpen = false;
  }

  async function pickChat(id: string, threadModelId?: string | null) {
    if (sending) return;
    selectedId = id;
    messages = [];
    messagesLoading = true;
    pendingSelectedId = id;
    cancelRename();
    resetThreadUi();
    err = null;
    void refreshMessages();
    void refreshImageGenAvailable();
    const mid = resolveLoadableModelId(models, [
      threadModelId,
      chats.find((c) => c.id === id)?.modelId,
    ]);
    if (mid && loadModelId !== mid) loadModelId = mid;
    else if (!mid && loadModelId) {
      try {
        await api.setChatModel(id, loadModelId);
        await refreshChatList();
      } catch (e) {
        err = String(e);
      }
    }
  }

  async function newChat(projectId?: string | null) {
    if (sending) return;
    err = null;
    const chatModelId = resolveLoadableModelId(models, [loadModelId]) || null;
    const c = await api.createChat(null, chatModelId, projectId ?? null);
    selectedId = c.id;
    messages = [];
    messagesLoading = true;
    pendingSelectedId = c.id;
    draft = "";
    pendingAttachments = [];
    cancelRename();
    resetThreadUi();
    await Promise.all([
      refreshChatList(),
      refreshMessages(),
      refreshModelsAndLoaded(),
    ]);
  }

  async function refreshModelsAndLoaded() {
    models = await api.listModels();
    const currentLoaded = await api.getLoadedLlm();
    loaded = currentLoaded;
    const lm = loadableModels(models);
    if (currentLoaded && lm.some((m) => m.id === currentLoaded.id)) {
      if (!loadModelId) {
        loadModelId = currentLoaded.id;
      } else if (!lm.some((m) => m.id === loadModelId)) {
        loadModelId = currentLoaded.id;
      }
    } else if (!loadModelId || !lm.some((m) => m.id === loadModelId)) {
      loadModelId = lm[0]?.id ?? "";
    }
    if (loadModelBusy && currentLoaded?.id === loadModelId) {
      loadModelInFlight = 0;
      loadModelBusy = false;
    }
  }

  let loadModelBusy = $state(false);
  /** Nested load calls (e.g. seq bumped mid-flight); busy stays true until all finish. */
  let loadModelInFlight = $state(0);
  /** Bumps on each load request so rapid model changes don’t apply a superseded result or flicker state. */
  let loadModelSeq = 0;

  const composerBlocked = $derived(sending || !loadModelId);
  const sendDisabled = $derived(
    composerBlocked || !selectedId || buildPayload() === null,
  );

  const runtimeStatusText = $derived.by(() => {
    const phase = runtimeStatus?.phase ?? "idle";
    if (phase === "loading") return "Loading local runtime...";
    if (phase === "ready") return "Local runtime ready";
    if (phase === "generating") return "Generating";
    if (phase === "failed") return runtimeStatus?.error ? `Runtime failed: ${runtimeStatus.error}` : "Runtime failed";
    if (loaded) return "Selected";
    return "No model selected";
  });

  const runtimeStatusTone = $derived.by(() => {
    const phase = runtimeStatus?.phase ?? "idle";
    if (phase === "ready") return "text-emerald-300";
    if (phase === "loading" || phase === "generating") return "text-lm-accent";
    if (phase === "failed") return "text-red-300";
    return "text-lm-muted";
  });

  async function refreshRuntimeStatus() {
    try {
      runtimeStatus = await api.getLlmRuntimeStatus();
    } catch {
      runtimeStatus = null;
    }
  }

  /** Activate the selected model immediately; local GGUF server startup happens on first send. */
  async function loadSelectedModel(force = false) {
    err = null;
    if (!loadModelId) return;
    const seq = ++loadModelSeq;
    loadModelInFlight += 1;
    loadModelBusy = true;
    try {
      const next = await api.loadLlm(loadModelId);
      if (seq !== loadModelSeq) return;
      loaded = next;
      void refreshRuntimeStatus();
      if (selectedId) {
        try {
          await api.setChatModel(selectedId, loadModelId);
          await refreshChatList();
          void refreshImageGenAvailable();
        } catch (e) {
          if (seq === loadModelSeq) err = String(e);
        }
      }
    } catch (e) {
      if (seq === loadModelSeq) err = String(e);
    } finally {
      loadModelInFlight = Math.max(0, loadModelInFlight - 1);
      loadModelBusy = loadModelInFlight > 0;
    }
  }

  let copyFlashKey = $state<string | null>(null);
  let copyFlashTimer: ReturnType<typeof setTimeout> | undefined;

  async function copyAssistantReply(key: string, raw: string) {
    const text = stripAssistantThinkNoise(raw).trim();
    if (!text) return;
    err = null;
    try {
      await navigator.clipboard.writeText(text);
      copyFlashKey = key;
      clearTimeout(copyFlashTimer);
      copyFlashTimer = setTimeout(() => {
        copyFlashKey = null;
      }, 2000);
    } catch (e) {
      err = `Copy failed: ${e}`;
    }
  }

  function buildPayload(): OutgoingUserPayload | null {
    return buildUserMessagePayload(draft, pendingAttachments);
  }

  function attachmentIcon(kind: string) {
    if (kind === "pdf") return FileText;
    return File;
  }

  function openEditModal(m: api.MessageDto) {
    editMessageId = m.id;
    editMessageRole = m.role === "assistant" ? "assistant" : "user";
    editMessageRaw = m.content;
    editDraft =
      m.role === "user" ? userMessageDisplay(m.content) : m.content;
    editModalOpen = true;
  }

  function closeEditModal() {
    editModalOpen = false;
    editMessageId = "";
    editMessageRaw = "";
    editDraft = "";
    editSaveBusy = false;
  }

  function prepareUserSave(raw: string, edited: string): string {
    const t = raw.trimStart();
    if (!t.startsWith("{")) return edited.trim();
    try {
      const j = JSON.parse(raw) as {
        _lm?: number;
        displayText?: string;
        display?: string;
      };
      if (j._lm === 1) {
        j.displayText = edited.trim();
        delete j.display;
        return JSON.stringify(j);
      }
    } catch {
      /* fall through */
    }
    return edited.trim();
  }

  async function saveEditFromModal() {
    if (!editMessageId || editSaveBusy) return;
    err = null;
    const trimmed =
      editMessageRole === "user"
        ? prepareUserSave(editMessageRaw, editDraft)
        : editDraft.trim();
    if (!trimmed) return;
    editSaveBusy = true;
    try {
      await api.updateMessage(editMessageId, trimmed);
      closeEditModal();
      await Promise.all([refreshMessages(), refreshChatList()]);
    } catch (e) {
      err = String(e);
    } finally {
      editSaveBusy = false;
    }
  }

  async function removeMessage(id: string) {
    if (!confirm("Delete this message and everything after it?")) return;
    err = null;
    try {
      await api.deleteMessage(id);
      await refreshMessages();
      await refreshChatList();
    } catch (e) {
      err = String(e);
    }
  }

  function ingestStreamChunk(raw: string) {
    if (raw === LM_STREAM_PHASE_USER_SAVED) {
      optimisticUser = null;
      void refreshMessages();
      return;
    }
    const toolPhase = parseLmToolPhase(raw);
    if (toolPhase) {
      streamToolLabel = toolPhase;
      streamInReasoning = false;
      return;
    }
    if (raw.includes("*Error:*")) {
      streamText += raw;
      streamInReasoning = false;
      streamToolLabel = null;
      return;
    }
    const parsed = parseLmStreamChunk(raw);
    if (!parsed) {
      streamText += raw;
      streamInReasoning = false;
      streamToolLabel = null;
      return;
    }
    if (parsed.t === "r") {
      streamInReasoning = true;
      streamToolLabel = null;
      return;
    }
    streamInReasoning = false;
    streamToolLabel = null;
    streamText += parsed.s;
  }

  function toggleThinking() {
    thinkingEnabled = !thinkingEnabled;
    try {
      localStorage.setItem(THINKING_LS_KEY, thinkingEnabled ? "1" : "0");
    } catch {
      /* ignore */
    }
  }

  function toggleWebSearch() {
    webSearchEnabled = !webSearchEnabled;
    try {
      localStorage.setItem(WEB_SEARCH_LS_KEY, webSearchEnabled ? "1" : "0");
    } catch {
      /* ignore */
    }
  }

  function toggleAgent() {
    agentEnabled = !agentEnabled;
    try {
      localStorage.setItem(AGENT_LS_KEY, agentEnabled ? "1" : "0");
    } catch {
      /* ignore */
    }
  }

  function toggleImageGen() {
    if (!imageGenAvailable) return;
    imageGenEnabled = !imageGenEnabled;
    try {
      localStorage.setItem(IMAGE_GEN_LS_KEY, imageGenEnabled ? "1" : "0");
    } catch {
      /* ignore */
    }
  }

  function toolLabelHuman(name: string): string {
    switch (name) {
      case "web_search":
        return "Searching the web…";
      case "generate_image":
        return "Generating image…";
      case "read_file":
        return "Reading file…";
      case "list_dir":
        return "Listing directory…";
      case "write_file":
        return "Writing file…";
      case "edit_file":
        return "Editing file…";
      case "create_folder":
        return "Creating folder…";
      case "run_command":
        return "Running command…";
      case "install_package":
        return "Installing package…";
      case "debug":
        return "Debugging…";
      case "system_info":
        return "Gathering system info…";
      default:
        return `Using ${name}…`;
    }
  }

  async function regenerateAssistant(assistantMessageId: string) {
    if (!selectedId || sending || composerBlocked) return;
    err = null;
    if (!(await ensureChatModelSynced())) {
      if (!effectiveModelId()) err = "Pick a model for this chat (header dropdown).";
      return;
    }
    await refreshImageGenAvailable();
    messages = messages.filter((m) => m.id !== assistantMessageId);
    sending = true;
    streamText = "";
    streamInReasoning = false;
    streamToolLabel = null;
    const headerModelId = effectiveModelId();
    try {
      await api.regenerateAssistantMessageStream(
        selectedId,
        assistantMessageId,
        thinkingEnabled,
        webSearchEnabled,
        agentEnabled,
        activeImageGenEnabled,
        ingestStreamChunk,
        headerModelId || null,
      );
      streamText = "";
      await Promise.all([refreshMessages(), refreshChatList()]);
    } catch (e) {
      err = String(e);
      await refreshMessages();
    } finally {
      sending = false;
      streamText = "";
      streamInReasoning = false;
      streamToolLabel = null;
    }
  }

  async function sendMessage() {
    if (sending || composerBlocked) return;
    const payload = buildPayload();
    if (!payload || !selectedId) return;
    err = null;
    sending = true;
    streamText = "";
    streamInReasoning = false;
    streamToolLabel = null;
    toolsMenuOpen = false;
    optimisticUser = payload;
    const selectedChatId = selectedId;
    const draftForContext = draft;
    draft = "";
    pendingAttachments = [];
    mentionOpen = false;
    mentionStart = -1;
    mentionQuery = "";
    scheduleContextUsageRefresh();
    if (!(await ensureChatModelSynced())) {
      if (!effectiveModelId()) err = "Pick a model for this chat (header dropdown).";
      sending = false;
      optimisticUser = null;
      return;
    }
    if (draftForContext.includes("@")) {
      await refreshContextDocs();
    }
    await refreshImageGenAvailable();
    const contextIds = resolveContextIdsFromDisplay(draftForContext);
    const headerModelId = effectiveModelId();
    try {
      await api.sendChatMessageStream(
        selectedChatId,
        payload.storageJson,
        thinkingEnabled,
        webSearchEnabled,
        agentEnabled,
        activeImageGenEnabled,
        ingestStreamChunk,
        contextIds,
        headerModelId || null,
      );
      optimisticUser = null;
      streamText = "";
      await Promise.all([refreshMessages(), refreshChatList()]);
    } catch (e) {
      err = String(e);
    } finally {
      sending = false;
      streamText = "";
      streamInReasoning = false;
      streamToolLabel = null;
      optimisticUser = null;
    }
  }

  function stopGenerating() {
    void api.stopGeneration();
  }

  function removeAttachment(i: number) {
    pendingAttachments = pendingAttachments.filter((_, j) => j !== i);
  }

  async function onFilesSelected(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const files = input.files;
    if (!files?.length) return;
    err = null;
    const next = [...pendingAttachments];
    for (const file of Array.from(files)) {
      try {
        next.push(await fileToPendingAttachment(file));
      } catch (e) {
        err = String(e);
        break;
      }
    }
    pendingAttachments = next;
    input.value = "";
  }

  function openFilePicker() {
    fileInputEl?.click();
  }

  onMount(() => {
    let cancelled = false;

    void (async () => {
      try {
        const sw = localStorage.getItem(SIDEBAR_W_LS_KEY);
        if (sw != null) {
          const n = parseInt(sw, 10);
          if (Number.isFinite(n)) sidebarWidth = clampSidebarWidth(n);
        }
        if (localStorage.getItem(SIDEBAR_COLLAPSED_LS_KEY) === "1") {
          sidebarCollapsed = true;
        }
      } catch {
        /* ignore */
      }
      try {
        const raw = localStorage.getItem(EXPANDED_PROJECTS_LS_KEY);
        if (raw) {
          const arr = JSON.parse(raw) as unknown;
          if (Array.isArray(arr)) {
            expandedProjectIds = new Set(
              arr.filter((x): x is string => typeof x === "string"),
            );
          }
        }
      } catch {
        /* ignore */
      }
      try {
        const v = localStorage.getItem(THINKING_LS_KEY);
        if (v != null) thinkingEnabled = v === "1" || v === "true";
        const ws = localStorage.getItem(WEB_SEARCH_LS_KEY);
        if (ws != null) webSearchEnabled = ws === "1" || ws === "true";
        const ag = localStorage.getItem(AGENT_LS_KEY);
        if (ag != null) agentEnabled = ag === "1" || ag === "true";
        const ig = localStorage.getItem(IMAGE_GEN_LS_KEY);
        if (ig != null) imageGenEnabled = ig === "1" || ig === "true";
      } catch {
        /* ignore */
      }
      void refreshRuntimeStatus();
      runtimeStatusTimer = setInterval(() => {
        void refreshRuntimeStatus();
      }, 1000);
      try {
        await Promise.all([refreshModelsAndLoaded(), refreshContextDocs()]);
        if (cancelled) return;
        await refreshChats();
        if (cancelled) return;
        const t = chats.find((c) => c.id === selectedId);
        const threadModel = resolveLoadableModelId(models, [t?.modelId]);
        if (threadModel && loadModelId !== threadModel) {
          loadModelId = threadModel;
        } else if (selectedId && loadModelId && !t?.modelId) {
          try {
            await api.setChatModel(selectedId, loadModelId);
            await refreshChatList();
          } catch (e) {
            if (!cancelled) err = String(e);
          }
        }
        if (!cancelled) void refreshImageGenAvailable();
      } catch (e) {
        if (!cancelled) err = String(e);
      }
    })();

    return () => {
      cancelled = true;
      if (contextUsageTimer !== undefined) clearTimeout(contextUsageTimer);
      if (runtimeStatusTimer !== undefined) clearInterval(runtimeStatusTimer);
    };
  });

  const thread = $derived(chats.find((c) => c.id === selectedId));

  const threadProject = $derived.by(() => {
    if (!thread?.projectId) return null;
    return projects.find((p) => p.id === thread.projectId) ?? null;
  });

  const showStreamGenerating = $derived(
    sending && streamText.length === 0 && !streamInReasoning && !streamToolLabel,
  );

  const uiLockedForGeneration = $derived(sending);
</script>

<div class="relative flex h-full min-h-0 flex-1 flex-col bg-lm-bg">
  <input
    bind:this={fileInputEl}
    type="file"
    multiple
    accept={CHAT_ATTACHMENT_ACCEPT}
    class="sr-only"
    aria-hidden="true"
    onchange={(e) => void onFilesSelected(e)}
  />

  {#if loadModelBusy}
    <div
      class="shrink-0 border-b border-lm-border/80 bg-lm-elevated/95 px-5 py-3.5 text-center text-base text-lm-muted"
    >
      Activating model...
    </div>
  {/if}
  {#if err}
    <div
      class="shrink-0 border-b border-red-900/40 bg-red-950/40 px-5 py-3.5 text-center text-base text-red-200"
    >
      {err}
    </div>
  {/if}

  <div
    class="relative flex min-h-0 flex-1 flex-col overflow-hidden lg:flex-row"
  >
    {#if !sidebarCollapsed}
      <aside
        class="lm-chat-sidebar relative flex max-h-56 shrink-0 flex-col overflow-hidden border-b border-lm-border/80 bg-lm-surface/50 lg:max-h-none lg:border-b-0 lg:border-r lg:border-lm-border/80"
        style="--lm-sidebar-w: {sidebarWidth}px;"
        aria-label="Conversation list"
      >
        <div
          class="flex items-center justify-between gap-2 border-b border-lm-border/80 px-5 py-3.5"
        >
          <div
            class="text-sm font-bold tracking-wide text-lm-muted uppercase"
          >
            Conversations
          </div>
          <button
            type="button"
            class="inline-flex h-11 w-11 shrink-0 items-center justify-center rounded-lg border border-lm-border bg-lm-surface text-lm-muted hover:bg-lm-surface-hover hover:text-lm-text"
            title="Hide conversation list"
            aria-label="Hide conversation list"
            onclick={() => toggleSidebarCollapsed()}
          >
            <PanelLeftClose class="size-5" strokeWidth={2} />
          </button>
        </div>
        <div class="shrink-0 border-b border-lm-border/80 p-2">
          <div class="flex gap-2">
            <button
              type="button"
              class="flex min-h-14 flex-1 items-center justify-center gap-2 rounded-xl border border-lm-accent/50 bg-lm-accent/15 px-3 py-3.5 text-base font-bold text-lm-accent hover:bg-lm-accent/25 disabled:pointer-events-none disabled:opacity-45"
              disabled={uiLockedForGeneration}
              title={uiLockedForGeneration ? "Stop generation before starting a new chat" : undefined}
              onclick={() => newChat()}
            >
              <Plus class="size-5" strokeWidth={2.5} />
              New chat
            </button>
            <button
              type="button"
              class="inline-flex h-14 w-14 shrink-0 items-center justify-center rounded-xl border border-lm-border bg-lm-surface text-lm-muted hover:bg-lm-surface-hover hover:text-lm-accent disabled:pointer-events-none disabled:opacity-45"
              disabled={uiLockedForGeneration}
              title="New project"
              aria-label="New project"
              onclick={() => openCreateProjectModal()}
            >
              <FolderPlus class="size-5" strokeWidth={2} />
            </button>
          </div>
        </div>
        <div class="min-h-0 flex-1 overflow-y-auto space-y-3 p-2">
          {#snippet chatRow(t: api.ChatDto, nested = false)}
            <div
              class="group flex gap-1 rounded-lg border transition-all duration-150 ease-out {nested
                ? 'border-lm-border/55 bg-lm-bg/55'
                : 'border-lm-border/70 bg-lm-surface/35'} {selectedId === t.id
                ? '!border-lm-accent/45 bg-lm-bg/90 shadow-sm'
                : 'hover:border-lm-border hover:bg-lm-bg/60'}"
            >
              {#if renamingId === t.id}
                <form
                  class="flex flex-1 flex-col gap-2 p-3"
                  onsubmit={(e) => {
                    e.preventDefault();
                    void commitRename();
                  }}
                >
                  <input
                    bind:value={renameDraft}
                    class="w-full rounded-xl border border-lm-border bg-lm-surface px-3 py-2 text-base"
                    aria-label="Chat title"
                  />
                  <div class="flex gap-2">
                    <button
                      type="submit"
                      class="min-h-14 rounded-xl border border-lm-accent bg-lm-accent px-4 py-2.5 text-base font-bold text-lm-bg"
                    >
                      Save
                    </button>
                    <button
                      type="button"
                      class="min-h-14 rounded-xl border border-lm-border px-4 py-2.5 text-base font-semibold"
                      onclick={() => cancelRename()}
                    >
                      Cancel
                    </button>
                  </div>
                </form>
              {:else}
                <button
                  type="button"
                  disabled={uiLockedForGeneration}
                  onclick={() => pickChat(t.id, t.modelId)}
                  class="min-w-0 flex-1 rounded-lg px-3 py-2 text-left transition-all duration-150 ease-out disabled:pointer-events-none disabled:opacity-45 {nested
                    ? 'py-2 text-sm'
                    : 'py-2.5 text-base'} {selectedId === t.id
                    ? 'font-semibold text-lm-text'
                    : 'text-lm-muted hover:text-lm-text'}"
                >
                  <div class="truncate font-semibold">{t.title}</div>
                  {#if !nested}
                    <div class="mt-0.5 truncate text-sm text-lm-muted transition-opacity duration-150">
                      {pendingSelectedId === t.id ? "Loading messages..." : (t.preview ?? "…")}
                    </div>
                  {/if}
                </button>
                <div
                  class="flex shrink-0 flex-col justify-center gap-1 pr-1 opacity-0 transition-opacity group-hover:opacity-100"
                >
                  <button
                    type="button"
                    disabled={uiLockedForGeneration}
                    class="rounded-lg border border-lm-border px-3 py-1.5 text-sm font-semibold text-lm-muted hover:text-lm-text disabled:opacity-40"
                    onclick={(e) => {
                      e.stopPropagation();
                      startRename(t);
                    }}
                  >
                    Rename
                  </button>
                  <button
                    type="button"
                    disabled={uiLockedForGeneration}
                    class="rounded-lg border border-red-900/40 px-3 py-1.5 text-sm font-semibold text-red-300 hover:bg-red-950/40 disabled:opacity-40"
                    onclick={(e) => {
                      e.stopPropagation();
                      void removeChat(t);
                    }}
                  >
                    Delete
                  </button>
                </div>
              {/if}
            </div>
          {/snippet}

          {#each projects as p (p.id)}
            {@const projectChats = chatsForProject(p.id)}
            {@const expanded = isProjectExpanded(p.id)}
            <section
              class="overflow-hidden rounded-xl border border-lm-border/80 bg-lm-surface/40 {expanded
                ? 'ring-1 ring-lm-border/40'
                : ''}"
              aria-label={`Project ${p.name}`}
            >
              <div
                class="flex items-center gap-1 border-b border-lm-border/50 px-2 py-1.5 {expanded
                  ? 'bg-lm-bg/30'
                  : ''}"
              >
                <button
                  type="button"
                  class="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-transparent text-lm-muted hover:border-lm-border/60 hover:bg-lm-surface-hover hover:text-lm-text"
                  aria-label={expanded ? "Collapse project" : "Expand project"}
                  aria-expanded={expanded}
                  onclick={() => toggleProjectExpanded(p.id)}
                >
                  {#if expanded}
                    <ChevronDown class="size-4" strokeWidth={2.5} />
                  {:else}
                    <ChevronRight class="size-4" strokeWidth={2.5} />
                  {/if}
                </button>
                <button
                  type="button"
                  class="min-w-0 flex-1 truncate text-left text-sm font-bold text-lm-text"
                  onclick={() => toggleProjectExpanded(p.id)}
                >
                  {p.name}
                </button>
                <span
                  class="shrink-0 rounded-md border border-lm-border/60 px-1.5 py-0.5 text-[0.65rem] tabular-nums text-lm-muted"
                  title="Chats in project"
                >
                  {projectChats.length}
                </span>
                <button
                  type="button"
                  class="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-transparent text-lm-muted hover:border-lm-border/60 hover:bg-lm-surface-hover hover:text-lm-accent disabled:opacity-40"
                  disabled={uiLockedForGeneration}
                  title="New chat in project"
                  aria-label="New chat in project"
                  onclick={(e) => {
                    e.stopPropagation();
                    void newChat(p.id);
                  }}
                >
                  <Plus class="size-4" strokeWidth={2.5} />
                </button>
                <button
                  type="button"
                  class="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-transparent text-lm-muted hover:border-lm-border/60 hover:bg-lm-surface-hover hover:text-lm-text disabled:opacity-40"
                  disabled={uiLockedForGeneration}
                  title="Project settings"
                  aria-label="Project settings"
                  onclick={(e) => {
                    e.stopPropagation();
                    openProjectSettings(p);
                  }}
                >
                  <Settings class="size-4" strokeWidth={2} />
                </button>
              </div>

              {#if expanded}
                <div class="px-2 py-2.5">
                  <nav
                    class="mb-2.5 flex min-w-0 items-center gap-0.5 px-1 text-xs text-lm-muted"
                    aria-label="Project location"
                  >
                    <span class="truncate font-semibold text-lm-text">{p.name}</span>
                    <ChevronRight class="size-3.5 shrink-0 opacity-50" strokeWidth={2.5} />
                    <span class="shrink-0 font-medium text-lm-muted">Chats</span>
                  </nav>

                  {#if projectChats.length === 0}
                    <p
                      class="rounded-lg border border-dashed border-lm-border/70 px-3 py-3 text-center text-xs text-lm-muted"
                    >
                      No chats yet — use <strong class="text-lm-text">+</strong> to add one.
                    </p>
                  {:else}
                    <ul
                      class="lm-project-tree relative ml-1 flex flex-col gap-2 border-l border-lm-border/75 pl-3"
                      role="list"
                    >
                      {#each projectChats as t (t.id)}
                        <li class="lm-project-tree-item relative pl-3">
                          {@render chatRow(t, true)}
                        </li>
                      {/each}
                    </ul>
                  {/if}
                </div>
              {/if}
            </section>
          {/each}

          {#if ungroupedChats.length > 0}
            <section
              class="rounded-xl border border-lm-border/80 bg-lm-surface/25 p-2"
              aria-label="Chats outside projects"
            >
              {#if projects.length > 0}
                <p
                  class="mb-2 px-1 text-xs font-semibold uppercase tracking-wide text-lm-muted"
                >
                  Other chats
                </p>
              {/if}
              <div class="flex flex-col gap-2.5">
                {#each ungroupedChats as t (t.id)}
                  <div class="min-w-0">
                    {@render chatRow(t, false)}
                  </div>
                {/each}
              </div>
            </section>
          {/if}

          {#if chats.length === 0 && projects.length === 0}
            <p class="px-2 py-4 text-sm text-lm-muted">No chats yet.</p>
          {/if}
        </div>
        <div
          class="absolute top-0 right-0 z-10 hidden h-full w-1.5 cursor-col-resize select-none hover:bg-lm-accent/25 lg:block {sidebarResizing ? 'bg-lm-accent/35' : ''}"
          role="separator"
          aria-orientation="vertical"
          aria-hidden="true"
          title="Drag to resize sidebar"
          onmousedown={(e) => onSidebarResizeStart(e)}
        ></div>
      </aside>
    {/if}

    <section class="flex min-h-0 min-w-0 flex-1 flex-col bg-lm-bg">
      {#if thread}
        <div
          class="flex shrink-0 flex-col gap-3 border-b border-lm-border/80 px-4 py-3.5 sm:px-5 xl:flex-row xl:items-start xl:justify-between"
        >
          <div class="flex w-full min-w-0 items-start gap-2 xl:max-w-[min(48rem,55vw)] xl:flex-1">
            {#if sidebarCollapsed}
              <button
                type="button"
                class="mt-0.5 inline-flex h-12 w-12 shrink-0 items-center justify-center rounded-xl border border-lm-border bg-lm-surface text-lm-muted hover:bg-lm-surface-hover hover:text-lm-text"
                title="Show conversation list"
                aria-label="Show conversation list"
                onclick={() => toggleSidebarCollapsed()}
              >
                <PanelLeft class="size-5" strokeWidth={2} />
              </button>
            {/if}
            <div class="min-w-0 flex-1">
              {#if threadProject}
                <nav
                  class="mb-1 flex min-w-0 items-center gap-1 text-sm text-lm-muted"
                  aria-label="Chat location"
                >
                  <span class="min-w-0 truncate font-medium text-lm-accent">{threadProject.name}</span>
                  <ChevronRight class="size-3.5 shrink-0 opacity-50" strokeWidth={2.5} />
                  <span class="min-w-0 truncate font-semibold text-lm-text">{thread.title}</span>
                </nav>
              {:else}
                <h2 class="truncate text-lg font-bold">{thread.title}</h2>
              {/if}
              <p class="mt-0.5 min-w-0 truncate text-sm text-lm-muted">
                {#if loadableModelsList.length === 0}
                  No models installed. Add one under Models.
                {:else if loaded}
                  <span class="font-medium text-lm-text/90">{loaded.name}</span>
                {:else}
                  Choose a model to start chatting.
                {/if}
              </p>
              <p class="mt-1 min-w-0 truncate text-xs font-semibold {runtimeStatusTone}">
                {runtimeStatusText}
              </p>
            </div>
          </div>
          <div
            class="grid w-full min-w-0 grid-cols-[minmax(0,1fr)_auto] gap-2 sm:grid-cols-[minmax(0,1fr)_auto_auto] xl:w-auto xl:max-w-[34rem] xl:shrink-0"
          >
            <label class="sr-only" for="lm-header-model-select">Model</label>
            <select
              id="lm-header-model-select"
              bind:value={loadModelId}
              disabled={loadModelBusy || uiLockedForGeneration}
              onchange={() => void onHeaderModelChange()}
              title={uiLockedForGeneration
                ? "Stop generation before changing model"
                : "Choose which model to use in this chat"}
              class="col-span-2 min-h-12 w-full min-w-0 rounded-xl border border-lm-border bg-lm-surface px-4 py-2.5 text-base font-medium transition-colors duration-150 focus:border-lm-accent/70 focus:outline-none disabled:opacity-50 sm:col-span-1"
            >
              {#each loadableModelsList as m (m.id)}
                <option value={m.id}>{m.name}</option>
              {/each}
            </select>
            <button
              type="button"
              class="min-h-12 shrink-0 rounded-xl border border-lm-accent bg-lm-accent px-4 py-2.5 text-base font-bold text-lm-bg transition-colors duration-150 hover:bg-lm-accent-hover disabled:pointer-events-none disabled:opacity-50"
              disabled={loadModelBusy || !loadModelId || uiLockedForGeneration}
              title="Use this model for the current chat"
              onclick={() => void loadSelectedModel(true)}
            >
              {loadModelBusy ? "Activating..." : "Use model"}
            </button>
            <button
              type="button"
              class="inline-flex h-12 w-12 shrink-0 items-center justify-center rounded-xl border border-lm-border bg-lm-surface text-lm-muted transition-colors duration-150 hover:bg-lm-surface-hover hover:text-lm-text disabled:pointer-events-none disabled:opacity-50"
              title="System prompt for this chat"
              aria-label="System prompt"
              disabled={uiLockedForGeneration || !selectedId}
              onclick={() => openSystemPromptModal()}
            >
              <NotepadText class="size-5" strokeWidth={2} />
            </button>
          </div>
        </div>

        <div
          class="min-h-0 flex-1 space-y-4 overflow-y-auto scroll-smooth overscroll-contain px-4 py-5 sm:px-6 [contain:layout]"
        >
          {#key selectedId}
          {#if messagesLoading && messages.length === 0}
            <div class="flex justify-start">
              <div
                class="w-full max-w-[min(100%,32rem)] rounded-2xl border border-lm-border/70 bg-lm-surface/50 px-5 py-4 text-base transition-all duration-200 ease-out sm:px-6 sm:py-5"
              >
                <div class="mb-3 flex items-center gap-2">
                  <Badge variant="muted">loading</Badge>
                  <span class="text-sm text-lm-muted">Opening chat...</span>
                </div>
                <div class="space-y-2">
                  <div class="h-3 w-11/12 rounded-full bg-lm-border/60"></div>
                  <div class="h-3 w-8/12 rounded-full bg-lm-border/45"></div>
                  <div class="h-3 w-5/12 rounded-full bg-lm-border/35"></div>
                </div>
              </div>
            </div>
          {/if}

          {#each messages as msg (msg.id)}
            <div class="flex {msg.role === 'user' ? 'justify-end' : 'justify-start'}">
              <div
              class="w-full max-w-[min(100%,48rem)] rounded-2xl border border-lm-border/80 px-4 py-4 text-base transition-all duration-200 ease-out sm:w-auto sm:px-6 sm:py-5 {msg.role === 'user'
                  ? 'border-lm-accent/35 bg-lm-accent-muted/20'
                  : 'bg-lm-surface/80'}"
              >
                <div class="mb-2 flex flex-wrap items-center gap-2">
                  <Badge variant={msg.role === "user" ? "accent" : "muted"}>{msg.role}</Badge>
                  <span class="text-sm text-lm-muted tabular-nums">
                    {new Date(msg.createdAt).toLocaleTimeString(undefined, {
                      hour: "2-digit",
                      minute: "2-digit",
                    })}
                  </span>
                  <span class="ml-auto flex flex-wrap gap-1">
                    <button
                      type="button"
                      class="rounded-md border border-lm-border px-2.5 py-1 text-sm font-semibold text-lm-muted hover:bg-lm-surface-hover hover:text-lm-text"
                      onclick={() => openEditModal(msg)}
                    >
                      Edit
                    </button>
                    <button
                      type="button"
                      class="rounded-md border border-red-900/40 px-2.5 py-1 text-sm font-semibold text-red-300 hover:bg-red-950/30"
                      onclick={() => removeMessage(msg.id)}
                    >
                      Delete
                    </button>
                    {#if msg.role === "assistant"}
                      <button
                        type="button"
                        class="inline-flex items-center gap-1 rounded-md border border-lm-border px-2.5 py-1 text-sm font-semibold text-lm-muted hover:bg-lm-surface-hover hover:text-lm-text"
                        title="Copy reply (markdown/plain text)"
                        onclick={() => void copyAssistantReply(msg.id, msg.content)}
                      >
                        {#if copyFlashKey === msg.id}
                          <Check class="size-3.5" strokeWidth={2.5} />
                          Copied
                        {:else}
                          <Clipboard class="size-3.5" strokeWidth={2} />
                          Copy
                        {/if}
                      </button>
                      <button
                        type="button"
                        class="rounded-md border border-lm-accent/40 px-2.5 py-1 text-sm font-semibold text-lm-accent hover:bg-lm-accent/10 disabled:opacity-40"
                        disabled={sending || composerBlocked}
                        onclick={() => regenerateAssistant(msg.id)}
                      >
                        Regenerate
                      </button>
                    {/if}
                  </span>
                </div>
                {#if msg.role === "assistant"}
                  <AssistantMarkdown raw={msg.content} />
                {:else}
                  {@const parts = parseUserMessageParts(msg.content)}
                  {#if parts.plain.trim()}
                    <div class="whitespace-pre-wrap leading-relaxed">{parts.plain}</div>
                  {/if}
                  {#if parts.attachments.length}
                    <div class="mt-2 flex flex-wrap gap-2">
                      {#each parts.attachments as att (att.name + att.kind)}
                        {@const Icon = attachmentIcon(att.kind)}
                        <span
                          class="inline-flex items-center gap-1.5 rounded-lg border border-lm-border bg-lm-bg px-3 py-1.5 text-sm font-medium text-lm-muted"
                          title={att.name}
                        >
                          <Icon class="size-3.5 shrink-0" strokeWidth={2} />
                          <span class="max-w-[10rem] truncate">{att.name}</span>
                        </span>
                      {/each}
                    </div>
                  {/if}
                  {#if !parts.plain.trim() && parts.attachments.length === 0}
                    <div class="whitespace-pre-wrap leading-relaxed">{userMessageDisplay(msg.content)}</div>
                  {/if}
                {/if}
              </div>
            </div>
          {/each}

          {#if optimisticUser}
            <div class="flex justify-end">
              <div
                class="w-full max-w-[min(100%,48rem)] rounded-2xl border border-lm-accent/35 bg-lm-accent-muted/20 px-4 py-4 text-base shadow-sm transition-all duration-200 ease-out sm:w-auto sm:px-6 sm:py-5"
              >
                <div class="mb-2 flex flex-wrap items-center gap-2">
                  <Badge variant="accent">user</Badge>
                  <span class="text-sm text-lm-muted">sending…</span>
                </div>
                {#if optimisticUser.displayText.trim()}
                  <div class="whitespace-pre-wrap leading-relaxed">
                    {optimisticUser.displayText}
                  </div>
                {/if}
                {#if optimisticUser.attachmentMetas.length}
                  <div class="mt-2 flex flex-wrap gap-2">
                    {#each optimisticUser.attachmentMetas as att (att.name + att.kind)}
                      {@const Icon = attachmentIcon(att.kind)}
                      <span
                        class="inline-flex items-center gap-1.5 rounded-lg border border-lm-border bg-lm-bg px-3 py-1.5 text-sm font-medium text-lm-muted"
                      >
                        <Icon class="size-3.5 shrink-0" strokeWidth={2} />
                        <span class="max-w-[10rem] truncate">{att.name}</span>
                      </span>
                    {/each}
                  </div>
                {/if}
              </div>
            </div>
          {/if}

          {#if showStreamGenerating}
            <div class="flex justify-start">
              <div
                class="w-full max-w-[min(100%,48rem)] rounded-2xl border border-lm-border/80 bg-lm-surface/80 px-4 py-4 text-base transition-all duration-200 ease-out sm:w-auto sm:px-6 sm:py-5"
              >
                <div class="flex flex-wrap items-center gap-2">
                  <Badge variant="muted">assistant</Badge>
                  <span class="text-sm font-medium text-lm-accent">Generating…</span>
                </div>
              </div>
            </div>
          {/if}

          {#if sending && streamInReasoning}
            <div class="flex justify-start">
              <div
                class="w-full max-w-[min(100%,48rem)] rounded-2xl border border-dashed border-lm-border/70 bg-lm-surface/40 px-4 py-4 text-base transition-all duration-200 ease-out sm:w-auto sm:px-6 sm:py-5"
              >
                <div class="flex flex-wrap items-center gap-2">
                  <Badge variant="muted">assistant</Badge>
                  <span class="text-sm font-medium italic text-lm-muted">Thinking…</span>
                </div>
              </div>
            </div>
          {/if}

          {#if sending && streamToolLabel}
            <div class="flex justify-start">
              <div
                class="w-full max-w-[min(100%,48rem)] rounded-2xl border border-dashed border-lm-accent/35 bg-lm-accent/5 px-4 py-4 text-base transition-all duration-200 ease-out sm:w-auto sm:px-6 sm:py-5"
              >
                <div class="flex flex-wrap items-center gap-2">
                  <Badge variant="muted">assistant</Badge>
                  <span class="text-sm font-medium italic text-lm-accent/90"
                    >{toolLabelHuman(streamToolLabel)}</span
                  >
                </div>
              </div>
            </div>
          {/if}

          {#if streamText}
            <div class="flex justify-start">
              <div
                class="w-full max-w-[min(100%,48rem)] rounded-2xl border border-lm-border/80 bg-lm-surface/80 px-4 py-4 text-base transition-all duration-200 ease-out sm:w-auto sm:px-6 sm:py-5"
              >
                <div class="mb-2 flex flex-wrap items-center gap-2">
                  <Badge variant="muted">assistant</Badge>
                  <span class="text-sm text-lm-muted">Reply</span>
                  <button
                    type="button"
                    class="ml-auto inline-flex items-center gap-1 rounded-md border border-lm-border px-2.5 py-1 text-sm font-semibold text-lm-muted hover:bg-lm-surface-hover hover:text-lm-text disabled:opacity-40"
                    disabled={!stripAssistantThinkNoise(streamText).trim()}
                    title="Copy partial reply so far"
                    onclick={() => void copyAssistantReply("_stream", streamText)}
                  >
                    {#if copyFlashKey === "_stream"}
                      <Check class="size-3.5" strokeWidth={2.5} />
                      Copied
                    {:else}
                      <Clipboard class="size-3.5" strokeWidth={2} />
                      Copy
                    {/if}
                  </button>
                </div>
                <AssistantMarkdown raw={streamText} />
              </div>
            </div>
          {/if}
          {/key}
        </div>

        <form
          class="shrink-0 border-t border-lm-border/80 bg-lm-surface/30 px-3 py-3 transition-colors duration-200 sm:px-5 sm:py-4"
          onsubmit={(e) => {
            e.preventDefault();
            void sendMessage();
          }}
        >
          {#if pendingAttachments.length}
            <div class="mb-2 flex flex-wrap gap-2">
              {#each pendingAttachments as a, i (i + a.name)}
                <button
                  type="button"
                  class="flex max-w-full items-center gap-2 rounded-full border border-lm-border bg-lm-bg px-4 py-1.5 text-sm font-medium hover:border-red-800/50 hover:text-red-200"
                  onclick={() => removeAttachment(i)}
                  title="Remove attachment"
                >
                  <span class="truncate"
                    >{a.name}{a.docType === "pdf" ? " · PDF" : " · file"}</span>
                  <span class="text-lm-muted">×</span>
                </button>
              {/each}
            </div>
          {/if}
          <div
            class="flex flex-col gap-2 rounded-2xl border border-lm-border/90 bg-lm-surface p-2 transition-all duration-200 ease-out focus-within:border-lm-accent/60 focus-within:bg-lm-surface-hover/35 focus-within:shadow-sm md:flex-row md:items-end"
          >
            <div class="flex flex-wrap items-center gap-1.5 md:shrink-0">
              <button
                type="button"
                class="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-lm-border bg-lm-surface text-lm-muted transition-colors duration-150 hover:bg-lm-surface-hover hover:text-lm-text disabled:pointer-events-none disabled:opacity-45 md:h-12 md:w-12"
                onclick={() => openFilePicker()}
                aria-label="Attach files"
                title="Attach PDF or text files"
                disabled={composerBlocked}
              >
                <Paperclip class="size-[1.35rem]" strokeWidth={2} />
              </button>
              <button
                type="button"
                class="flex h-11 shrink-0 items-center gap-1.5 rounded-xl border px-3 text-sm font-semibold transition-colors duration-150 md:h-12 {thinkingEnabled
                  ? 'border-lm-accent/45 bg-lm-accent/10 text-lm-accent hover:bg-lm-accent/18'
                  : 'border-lm-border bg-lm-surface text-lm-muted hover:bg-lm-surface-hover hover:text-lm-text'}"
                onclick={() => toggleThinking()}
                disabled={sending || composerBlocked}
                title={thinkingEnabled
                  ? "Thinking on: internal reasoning (label only in chat)."
                  : "Thinking off: faster direct answers when the model supports it."}
                aria-pressed={thinkingEnabled}
                aria-label={thinkingEnabled ? "Thinking on; click for off" : "Thinking off; click for on"}
              >
                <Brain class="size-4 shrink-0" strokeWidth={2} />
                <span class="max-w-[4.5rem] truncate sm:inline"
                  >{thinkingEnabled ? "Think" : "Fast"}</span
                >
              </button>
              <div class="relative shrink-0">
              {#if toolsMenuOpen}
                <div
                  class="absolute bottom-full left-0 z-30 mb-1.5 w-[min(18rem,calc(100vw-2rem))] rounded-xl border border-lm-border bg-lm-surface p-2 shadow-lg"
                  role="menu"
                  aria-label="Chat tools"
                >
                  <p class="px-2 pb-1.5 text-xs font-semibold uppercase tracking-wide text-lm-muted">
                    Tools
                  </p>
                  <button
                    type="button"
                    role="menuitemcheckbox"
                    aria-checked={webSearchEnabled}
                    class="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2.5 text-left text-sm hover:bg-lm-surface-hover {webSearchEnabled
                      ? 'text-lm-accent'
                      : 'text-lm-text'}"
                    onclick={() => toggleWebSearch()}
                  >
                    <Globe class="size-4 shrink-0" strokeWidth={2} />
                    <span class="min-w-0 flex-1">
                      <span class="block font-semibold">Web search</span>
                      <span class="block text-xs text-lm-muted"
                        >Model can search the web for current info</span
                      >
                    </span>
                    <span
                      class="shrink-0 rounded-md border px-2 py-0.5 text-xs font-bold {webSearchEnabled
                        ? 'border-lm-accent/50 bg-lm-accent/15 text-lm-accent'
                        : 'border-lm-border text-lm-muted'}"
                      >{webSearchEnabled ? "On" : "Off"}</span
                    >
                  </button>
                  <button
                    type="button"
                    role="menuitemcheckbox"
                    aria-checked={agentEnabled}
                    class="mt-0.5 flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2.5 text-left text-sm hover:bg-lm-surface-hover {agentEnabled
                      ? 'text-lm-accent'
                      : 'text-lm-text'}"
                    onclick={() => toggleAgent()}
                  >
                    <Terminal class="size-4 shrink-0" strokeWidth={2} />
                    <span class="min-w-0 flex-1">
                      <span class="block font-semibold">Agent</span>
                      <span class="block text-xs text-lm-muted"
                        >Run commands, edit files, create folders, install packages, debug</span
                      >
                    </span>
                    <span
                      class="shrink-0 rounded-md border px-2 py-0.5 text-xs font-bold {agentEnabled
                        ? 'border-lm-accent/50 bg-lm-accent/15 text-lm-accent'
                        : 'border-lm-border text-lm-muted'}"
                      >{agentEnabled ? "On" : "Off"}</span
                    >
                  </button>
                  <button
                    type="button"
                    role="menuitemcheckbox"
                    aria-checked={imageGenEnabled}
                    disabled={!imageGenAvailable}
                    class="mt-0.5 flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2.5 text-left text-sm hover:bg-lm-surface-hover disabled:cursor-not-allowed disabled:opacity-45 {activeImageGenEnabled
                      ? 'text-lm-accent'
                      : 'text-lm-text'}"
                    onclick={() => toggleImageGen()}
                  >
                    <ImagePlus class="size-4 shrink-0" strokeWidth={2} />
                    <span class="min-w-0 flex-1">
                      <span class="block font-semibold">Image generation</span>
                      <span class="block text-xs text-lm-muted">{imageGenToolsHint}</span>
                    </span>
                    <span
                      class="shrink-0 rounded-md border px-2 py-0.5 text-xs font-bold {imageGenEnabled &&
                      imageGenAvailable
                        ? 'border-lm-accent/50 bg-lm-accent/15 text-lm-accent'
                        : 'border-lm-border text-lm-muted'}"
                      >{imageGenEnabled && imageGenAvailable ? "On" : "Off"}</span
                    >
                  </button>
                </div>
              {/if}
                <button
                  type="button"
                  class="flex h-11 shrink-0 items-center gap-1.5 rounded-xl border px-3 text-sm font-semibold transition-colors duration-150 md:h-12 {webSearchEnabled ||
                agentEnabled ||
                (imageGenEnabled && imageGenAvailable)
                  ? 'border-lm-accent/45 bg-lm-accent/10 text-lm-accent hover:bg-lm-accent/18'
                  : 'border-lm-border bg-lm-surface text-lm-muted hover:bg-lm-surface-hover hover:text-lm-text'}"
                  onclick={() => (toolsMenuOpen = !toolsMenuOpen)}
                  disabled={sending || composerBlocked}
                  title="Tools: web search and local agent"
                  aria-expanded={toolsMenuOpen}
                  aria-haspopup="menu"
                  aria-label="Chat tools"
                >
                  <Wrench class="size-4 shrink-0" strokeWidth={2} />
                  <span class="max-w-[4.5rem] truncate sm:inline">Tools</span>
                </button>
              </div>
            </div>
            <div class="relative min-h-14 w-full min-w-0 flex-1">
              {#if mentionOpen && mentionCandidates.length > 0}
                <div
                  class="absolute bottom-full left-0 z-20 mb-1 max-h-48 w-[min(100%,22rem)] overflow-y-auto rounded-xl border border-lm-border bg-lm-surface py-1"
                  role="listbox"
                  aria-label="Reference files"
                >
                  {#each mentionCandidates as doc, i (doc.id)}
                    <button
                      type="button"
                      role="option"
                      aria-selected={i === mentionPickIndex}
                      class="flex w-full px-3 py-2 text-left text-sm {i === mentionPickIndex
                        ? 'bg-lm-accent/20 text-lm-text'
                        : 'text-lm-muted hover:bg-lm-surface-hover hover:text-lm-text'}"
                      onclick={() => void pickMention(doc)}
                    >
                      <span class="truncate font-medium">{doc.name}</span>
                    </button>
                  {/each}
                </div>
              {:else if mentionOpen && contextDocs.length === 0}
                <div
                  class="absolute bottom-full left-0 z-20 mb-1 max-w-[20rem] rounded-xl border border-lm-border bg-lm-surface px-4 py-2.5 text-sm text-lm-muted"
                >
                  No reference files yet. Add them under <strong>Context</strong> in the app.
                </div>
              {:else if mentionOpen && mentionCandidates.length === 0 && mentionQuery.trim()}
                <div
                  class="absolute bottom-full left-0 z-20 mb-1 max-w-[20rem] rounded-xl border border-lm-border bg-lm-surface px-4 py-2.5 text-sm text-lm-muted"
                >
                  No matching reference file. Check the name after <strong>@</strong>.
                </div>
              {/if}
              <textarea
                bind:this={textareaRef}
                bind:value={draft}
                rows={1}
                placeholder="Message… Type @ for reference files · Shift+Enter newline"
                disabled={composerBlocked}
                class="max-h-36 min-h-14 w-full resize-none overflow-y-auto rounded-xl border border-transparent bg-lm-bg/35 px-3 py-3 text-base leading-snug text-lm-text transition-colors duration-150 placeholder:text-lm-muted focus:border-lm-accent/35 focus:bg-lm-bg/55 focus:ring-0 focus:outline-none disabled:opacity-50 sm:max-h-44"
                oninput={() => {
                  updateMentionUi();
                  scheduleContextUsageRefresh();
                }}
                onselect={() => updateMentionUi()}
                onkeydown={(e) => {
                  onDraftKeydown(e);
                  if (e.defaultPrevented) return;
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    void sendMessage();
                  }
                }}
              ></textarea>
            </div>
            <div class="flex shrink-0 items-center justify-end gap-1.5">
              {#if contextUsage}
                <ContextRing
                  percent={contextUsage.usedPercent}
                  size={28}
                  details={contextRingDetails}
                />
              {/if}
              {#if sending}
                <button
                  type="button"
                  class="flex h-12 shrink-0 items-center gap-1.5 rounded-xl border border-amber-800/50 bg-amber-950/35 px-4 text-base font-bold text-amber-100 transition-colors duration-150 hover:bg-amber-950/50"
                  onclick={() => stopGenerating()}
                  title="Stop generating"
                >
                  <Square class="size-3.5 fill-current" strokeWidth={2} />
                  Stop
                </button>
              {:else}
                <button
                  type="submit"
                  class="h-12 min-w-[5.5rem] shrink-0 rounded-xl border border-lm-accent bg-lm-accent px-5 text-base font-bold text-lm-bg transition-colors duration-150 hover:bg-lm-accent-hover disabled:pointer-events-none disabled:opacity-45"
                  disabled={sendDisabled}
                >
                  Send
                </button>
              {/if}
            </div>
          </div>
        </form>
      {:else}
        <div class="flex flex-1 items-center justify-center px-4 text-base text-lm-muted">
          Select or create a chat
        </div>
      {/if}
    </section>
  </div>
</div>

{#if editModalOpen}
  <div
    class="fixed inset-0 z-[100] flex items-center justify-center bg-black/55 p-4"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget && !editSaveBusy) closeEditModal();
    }}
  >
    <div
      class="max-h-[min(90vh,40rem)] w-full max-w-2xl overflow-hidden rounded-2xl border border-lm-border bg-lm-surface"
      role="dialog"
      tabindex="-1"
      aria-modal="true"
      aria-labelledby="lm-edit-message-title"
      onkeydown={(e) => {
        if (e.key === "Escape" && !editSaveBusy) {
          e.preventDefault();
          closeEditModal();
        }
      }}
    >
      <div class="border-b border-lm-border px-5 py-4">
        <h2 id="lm-edit-message-title" class="text-lg font-bold text-lm-text">
          {editMessageRole === "assistant" ? "Edit assistant reply" : "Edit message"}
        </h2>
        <p class="mt-1 text-sm text-lm-muted">
          {editMessageRole === "assistant"
            ? "Edit the markdown/plain text of this reply. Changes are saved to the chat history."
            : "Edit your message text. Attachments are kept; only the visible text changes."}
        </p>
      </div>
      <div class="max-h-[min(60vh,28rem)] overflow-y-auto px-5 py-4">
        <label class="sr-only" for="lm-edit-message-input">Message content</label>
        <textarea
          id="lm-edit-message-input"
          bind:value={editDraft}
          rows="12"
          disabled={editSaveBusy}
          class="min-h-[16rem] w-full rounded-xl border border-lm-border bg-lm-bg px-4 py-3 font-mono text-sm leading-relaxed text-lm-text disabled:opacity-50"
          placeholder="Message content…"
        ></textarea>
      </div>
      <div class="flex flex-wrap justify-end gap-2 border-t border-lm-border px-5 py-4">
        <button
          type="button"
          class="rounded-xl border border-lm-border px-5 py-2.5 text-base font-semibold text-lm-muted hover:bg-lm-bg disabled:opacity-50"
          disabled={editSaveBusy}
          onclick={() => closeEditModal()}
        >
          Cancel
        </button>
        <button
          type="button"
          class="rounded-xl border border-lm-accent bg-lm-accent px-5 py-2.5 text-base font-bold text-lm-bg hover:bg-lm-accent-hover disabled:opacity-50"
          disabled={editSaveBusy || !editDraft.trim()}
          onclick={() => void saveEditFromModal()}
        >
          {editSaveBusy ? "Saving…" : "Save"}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if systemPromptOpen}
  <div
    class="fixed inset-0 z-[100] flex items-center justify-center bg-black/55 p-4"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeSystemPromptModal();
    }}
  >
    <div
      class="max-h-[min(90vh,40rem)] w-full max-w-lg overflow-hidden rounded-2xl border border-lm-border bg-lm-surface"
      role="dialog"
      tabindex="-1"
      aria-modal="true"
      aria-labelledby="lm-system-prompt-title"
      onkeydown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          closeSystemPromptModal();
        }
      }}
    >
      <div class="border-b border-lm-border px-5 py-4">
        <h2 id="lm-system-prompt-title" class="text-lg font-bold text-lm-text">
          System prompt
        </h2>
        <p class="mt-1 text-sm text-lm-muted">
          Sent as an extra system message at the start of this chat only. Leave empty for
          default behavior.
        </p>
      </div>
      <div class="max-h-[min(60vh,24rem)] overflow-y-auto px-5 py-4">
        <label class="sr-only" for="lm-system-prompt-input">Instructions</label>
        <textarea
          id="lm-system-prompt-input"
          bind:value={systemPromptDraft}
          rows="10"
          class="min-h-[13rem] w-full rounded-xl border border-lm-border bg-lm-bg px-4 py-3 text-base text-lm-text"
          placeholder="Optional instructions for the model in this thread…"
        ></textarea>
      </div>
      <div class="flex flex-wrap justify-end gap-2 border-t border-lm-border px-5 py-4">
        <button
          type="button"
          class="rounded-xl border border-lm-border px-5 py-2.5 text-base font-semibold text-lm-muted hover:bg-lm-bg"
          onclick={() => closeSystemPromptModal()}
        >
          Cancel
        </button>
        <button
          type="button"
          class="rounded-xl border border-lm-accent bg-lm-accent px-5 py-2.5 text-base font-bold text-lm-bg hover:bg-lm-accent-hover"
          onclick={() => void saveSystemPrompt()}
        >
          Save
        </button>
      </div>
    </div>
  </div>
{/if}

{#if createProjectOpen}
  <div
    class="fixed inset-0 z-[100] flex items-center justify-center bg-black/55 p-4"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeCreateProjectModal();
    }}
  >
    <div
      class="w-full max-w-lg overflow-hidden rounded-2xl border border-lm-border bg-lm-surface"
      role="dialog"
      tabindex="-1"
      aria-modal="true"
      aria-labelledby="lm-create-project-title"
      onkeydown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          closeCreateProjectModal();
        }
      }}
    >
      <div class="border-b border-lm-border px-5 py-4">
        <h2 id="lm-create-project-title" class="text-lg font-bold text-lm-text">
          New project
        </h2>
        <p class="mt-1 text-sm text-lm-muted">
          Group related chats and share project context across them.
        </p>
      </div>
      <div class="space-y-4 px-5 py-4">
        <div>
          <label class="block text-sm font-semibold text-lm-muted" for="lm-project-name">Name</label>
          <input
            id="lm-project-name"
            type="text"
            bind:value={createProjectName}
            placeholder="My project"
            class="mt-1 w-full rounded-xl border border-lm-border bg-lm-bg px-4 py-2.5 text-base"
          />
        </div>
        <div>
          <label class="block text-sm font-semibold text-lm-muted" for="lm-project-desc">Description</label>
          <textarea
            id="lm-project-desc"
            bind:value={createProjectDesc}
            rows="3"
            placeholder="Optional summary"
            class="mt-1 w-full rounded-xl border border-lm-border bg-lm-bg px-4 py-2.5 text-base"
          ></textarea>
        </div>
        <div>
          <label class="block text-sm font-semibold text-lm-muted" for="lm-project-tags">Tags</label>
          <input
            id="lm-project-tags"
            type="text"
            bind:value={createProjectTags}
            placeholder="research, api, v2 (comma-separated)"
            class="mt-1 w-full rounded-xl border border-lm-border bg-lm-bg px-4 py-2.5 text-base"
          />
        </div>
      </div>
      <div class="flex flex-wrap justify-end gap-2 border-t border-lm-border px-5 py-4">
        <button
          type="button"
          class="rounded-xl border border-lm-border px-5 py-2.5 text-base font-semibold text-lm-muted hover:bg-lm-bg"
          onclick={() => closeCreateProjectModal()}
        >
          Cancel
        </button>
        <button
          type="button"
          class="rounded-xl border border-lm-accent bg-lm-accent px-5 py-2.5 text-base font-bold text-lm-bg hover:bg-lm-accent-hover disabled:opacity-50"
          disabled={createProjectBusy || !createProjectName.trim()}
          onclick={() => void saveCreateProject()}
        >
          {createProjectBusy ? "Creating…" : "Create project"}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if projectSettingsOpen}
  <div
    class="fixed inset-0 z-[100] flex items-center justify-center bg-black/55 p-4"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeProjectSettings();
    }}
  >
    <div
      class="max-h-[min(90vh,44rem)] w-full max-w-lg overflow-hidden rounded-2xl border border-lm-border bg-lm-surface"
      role="dialog"
      tabindex="-1"
      aria-modal="true"
      aria-labelledby="lm-project-settings-title"
      onkeydown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          closeProjectSettings();
        }
      }}
    >
      <div class="border-b border-lm-border px-5 py-4">
        <h2 id="lm-project-settings-title" class="text-lg font-bold text-lm-text">
          Project settings
        </h2>
        <p class="mt-1 text-sm text-lm-muted">
          Project context is injected into every chat in this project (in addition to each chat’s system prompt).
        </p>
      </div>
      <div class="max-h-[min(60vh,28rem)] space-y-4 overflow-y-auto px-5 py-4">
        <div>
          <label class="block text-sm font-semibold text-lm-muted" for="lm-ps-name">Name</label>
          <input
            id="lm-ps-name"
            type="text"
            bind:value={projectSettingsName}
            class="mt-1 w-full rounded-xl border border-lm-border bg-lm-bg px-4 py-2.5 text-base"
          />
        </div>
        <div>
          <label class="block text-sm font-semibold text-lm-muted" for="lm-ps-desc">Description</label>
          <textarea
            id="lm-ps-desc"
            bind:value={projectSettingsDesc}
            rows="2"
            class="mt-1 w-full rounded-xl border border-lm-border bg-lm-bg px-4 py-2.5 text-base"
          ></textarea>
        </div>
        <div>
          <label class="block text-sm font-semibold text-lm-muted" for="lm-ps-tags">Tags</label>
          <input
            id="lm-ps-tags"
            type="text"
            bind:value={projectSettingsTags}
            placeholder="comma-separated"
            class="mt-1 w-full rounded-xl border border-lm-border bg-lm-bg px-4 py-2.5 text-base"
          />
        </div>
        <div>
          <label class="block text-sm font-semibold text-lm-muted" for="lm-ps-context">Project context</label>
          <textarea
            id="lm-ps-context"
            bind:value={projectSettingsContext}
            rows="8"
            placeholder="Background, goals, conventions, key facts…"
            class="mt-1 min-h-[10rem] w-full rounded-xl border border-lm-border bg-lm-bg px-4 py-3 text-base"
          ></textarea>
        </div>
      </div>
      <div class="flex flex-wrap items-center justify-between gap-2 border-t border-lm-border px-5 py-4">
        <button
          type="button"
          class="rounded-xl border border-red-900/50 px-4 py-2.5 text-base font-semibold text-red-200 hover:bg-red-950/35 disabled:opacity-50"
          disabled={projectSettingsBusy}
          onclick={() => {
            const p = projects.find((x) => x.id === projectSettingsId);
            if (p) void removeProject(p);
            closeProjectSettings();
          }}
        >
          Delete project
        </button>
        <div class="flex gap-2">
          <button
            type="button"
            class="rounded-xl border border-lm-border px-5 py-2.5 text-base font-semibold text-lm-muted hover:bg-lm-bg"
            onclick={() => closeProjectSettings()}
          >
            Cancel
          </button>
          <button
            type="button"
            class="rounded-xl border border-lm-accent bg-lm-accent px-5 py-2.5 text-base font-bold text-lm-bg hover:bg-lm-accent-hover disabled:opacity-50"
            disabled={projectSettingsBusy || !projectSettingsName.trim()}
            onclick={() => void saveProjectSettings()}
          >
            {projectSettingsBusy ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  @media (min-width: 1024px) {
    .lm-chat-sidebar {
      width: var(--lm-sidebar-w, 280px);
      max-width: min(100%, 480px);
    }
  }

  /* Tree branch: horizontal connector into each nested chat row */
  .lm-project-tree-item::before {
    content: "";
    position: absolute;
    left: -0.75rem;
    top: 50%;
    width: 0.75rem;
    height: 1px;
    background: color-mix(in srgb, var(--color-lm-border, #333) 75%, transparent);
  }
</style>
