<script lang="ts">
  import { onMount } from "svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import * as api from "$lib/tauri-bridge";

  let modelsDir = $state("");
  let appDataDir = $state("");

  let paths = $state<api.PathsDto | null>(null);
  let saveMsg = $state<string | null>(null);

  /** llama-server / generation (SQLite settings keys) */
  let nCtx = $state(4096);
  let nThreads = $state(0);
  let nGpuLayers = $state(0);
  let maxTokens = $state(768);
  let temperature = $state(0.7);
  let topP = $state(0.9);
  let seed = $state(1234);

  let headlessStatus = $state<api.HeadlessServerStatusDto | null>(null);
  let headlessBusy = $state(false);
  let headlessMsg = $state<string | null>(null);
  let headlessHost = $state("127.0.0.1");
  let headlessPort = $state(11436);
  let headlessDataDir = $state("");
  let headlessModelsDir = $state("");
  let headlessRuntimeDir = $state("");
  let headlessAuthMode = $state<"none" | "bearer">("none");
  let headlessApiKey = $state("");

  const ctxPresets = [
    [4096, "4k"],
    [8192, "8k"],
    [16384, "16k"],
    [32768, "32k"],
    [65536, "64k"],
    [131072, "128k"],
  ] as const;

  function mapStorageFromSettings(rows: [string, string][]) {
    const m = Object.fromEntries(rows);
    modelsDir = m.models_dir ?? "";
  }

  function mapInferenceFromSettings(rows: [string, string][]) {
    const m = Object.fromEntries(rows);
    nCtx = clampInt(m.n_ctx, 4096, 512, 262_144);
    nThreads = clampInt(m.n_threads, 0, 0, 256);
    nGpuLayers = clampInt(m.n_gpu_layers, 0, 0, 999);
    maxTokens = clampInt(m.max_tokens, 768, 16, 32_768);
    temperature = clampNum(m.temperature, 0.7, 0, 2);
    topP = clampNum(m.top_p, 0.9, 0.05, 1);
    seed = clampInt(m.seed, 1234, 0, 4_294_967_295);
  }

  function clampInt(s: string | undefined, fallback: number, min: number, max: number): number {
    const n = parseInt(String(s ?? ""), 10);
    if (!Number.isFinite(n)) return fallback;
    return Math.min(max, Math.max(min, n));
  }

  function clampNum(s: string | undefined, fallback: number, min: number, max: number): number {
    const n = parseFloat(String(s ?? ""));
    if (!Number.isFinite(n)) return fallback;
    return Math.min(max, Math.max(min, n));
  }

  function randomApiKey(): string {
    const bytes = new Uint8Array(24);
    crypto.getRandomValues(bytes);
    return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  }

  function applyHeadlessStatus(status: api.HeadlessServerStatusDto) {
    headlessStatus = status;
    headlessHost = status.host || headlessHost || "127.0.0.1";
    headlessPort = clampInt(String(status.port), headlessPort || 11436, 1, 65535);
    headlessAuthMode = status.authMode === "bearer" ? "bearer" : "none";
    headlessDataDir = status.dataDir || appDataDir;
    headlessModelsDir = status.modelsDir || "";
    headlessRuntimeDir = status.runtimeDir || "";
  }

  async function refreshHeadlessServer(showMessage = false) {
    headlessBusy = true;
    if (showMessage) headlessMsg = null;
    try {
      const status = await api.getHeadlessServerStatus();
      applyHeadlessStatus(status);
      if (showMessage) headlessMsg = "API server status refreshed.";
    } catch (e) {
      headlessMsg = String(e);
    } finally {
      headlessBusy = false;
    }
  }

  onMount(async () => {
    const rows = await api.getSettings();
    mapStorageFromSettings(rows);
    mapInferenceFromSettings(rows);
    try {
      paths = await api.getPaths();
      appDataDir = paths.configuredAppDataDir;
    } catch {
      paths = null;
    }
    await refreshHeadlessServer();
  });

  async function browseModelsDir() {
    const dir = await open({ directory: true, multiple: false });
    if (dir === null) return;
    const d = Array.isArray(dir) ? dir[0] : dir;
    if (d) modelsDir = d;
  }

  async function browseAppDataDir() {
    const dir = await open({ directory: true, multiple: false });
    if (dir === null) return;
    const d = Array.isArray(dir) ? dir[0] : dir;
    if (d) appDataDir = d;
  }

  async function browseHeadlessDataDir() {
    const dir = await open({ directory: true, multiple: false });
    if (dir === null) return;
    const d = Array.isArray(dir) ? dir[0] : dir;
    if (d) headlessDataDir = d;
  }

  async function browseHeadlessModelsDir() {
    const dir = await open({ directory: true, multiple: false });
    if (dir === null) return;
    const d = Array.isArray(dir) ? dir[0] : dir;
    if (d) headlessModelsDir = d;
  }

  async function browseHeadlessRuntimeDir() {
    const dir = await open({ directory: true, multiple: false });
    if (dir === null) return;
    const d = Array.isArray(dir) ? dir[0] : dir;
    if (d) headlessRuntimeDir = d;
  }

  async function save() {
    saveMsg = null;
    try {
      const ctx = Math.min(262_144, Math.max(512, Math.round(Number(nCtx)) || 4096));
      const threads = Math.min(256, Math.max(0, Math.round(Number(nThreads)) || 0));
      const ngl = Math.min(999, Math.max(0, Math.round(Number(nGpuLayers)) || 0));
      const maxT = Math.min(32_768, Math.max(16, Math.round(Number(maxTokens)) || 768));
      const temp = Math.min(2, Math.max(0, Number(temperature) || 0.7));
      const tp = Math.min(1, Math.max(0.05, Number(topP) || 0.9));
      const sd = Math.min(4_294_967_295, Math.max(0, Math.round(Number(seed)) || 1234));

      nCtx = ctx;
      nThreads = threads;
      nGpuLayers = ngl;
      maxTokens = maxT;
      temperature = temp;
      topP = tp;
      seed = sd;

      await api.setSetting("models_dir", modelsDir);
      await api.setSetting("app_data_dir", appDataDir.trim());
      await api.setSetting("n_ctx", String(ctx));
      await api.setSetting("n_threads", String(threads));
      await api.setSetting("n_gpu_layers", String(ngl));
      await api.setSetting("max_tokens", String(maxT));
      await api.setSetting("temperature", String(temp));
      await api.setSetting("top_p", String(tp));
      await api.setSetting("seed", String(sd));
      paths = await api.getPaths();
      appDataDir = paths.configuredAppDataDir;
      saveMsg =
        "Saved. For context or thread changes to apply to the AI server, use Load model again on the Chat screen. If you changed app data location, fully quit and reopen LocalMOD.";
    } catch (e) {
      saveMsg = String(e);
    }
  }

  async function startStandaloneServer() {
    headlessBusy = true;
    headlessMsg = null;
    try {
      const port = clampInt(String(headlessPort), 11436, 1, 65535);
      const dataDir = headlessDataDir.trim() || appDataDir.trim();
      headlessPort = port;
      headlessDataDir = dataDir;
      const status = await api.startHeadlessServer({
        host: headlessHost.trim() || "127.0.0.1",
        port,
        dataDir,
        modelsDir: headlessModelsDir.trim(),
        runtimeDir: headlessRuntimeDir.trim(),
        authMode: headlessAuthMode,
        apiKey: headlessApiKey,
      });
      applyHeadlessStatus(status);
      headlessMsg = `API server is on at ${status.baseUrl}`;
    } catch (e) {
      headlessMsg = String(e);
    } finally {
      headlessBusy = false;
    }
  }

  async function stopStandaloneServer() {
    headlessBusy = true;
    headlessMsg = null;
    try {
      const status = await api.stopHeadlessServer();
      applyHeadlessStatus(status);
      headlessMsg = "API server is off.";
    } catch (e) {
      headlessMsg = String(e);
    } finally {
      headlessBusy = false;
    }
  }

  async function copyHeadlessBaseUrl() {
    const baseUrl = headlessStatus?.baseUrl || `http://${headlessHost}:${headlessPort}/v1`;
    try {
      await navigator.clipboard.writeText(baseUrl);
      headlessMsg = "API URL copied.";
    } catch (e) {
      headlessMsg = String(e);
    }
  }
</script>

<div class="flex-1 overflow-y-auto overscroll-y-contain">
  <div class="mx-auto max-w-2xl px-4 py-8 sm:px-6 md:py-10">
    <PageHeader
      title="Storage"
      description="Choose where your models and app data are kept."
    />

    {#if saveMsg}
      <p
        class="mb-6 rounded-md border border-lm-border bg-lm-elevated px-4 py-3 text-sm text-lm-text"
      >
        {saveMsg}
      </p>
    {/if}

    <div class="space-y-6">
      <Card title="Model files">
        <p class="mb-4 text-sm text-lm-muted">
          Downloaded and imported models are read from this folder.
        </p>
        <div class="flex gap-2">
          <input
            type="text"
            bind:value={modelsDir}
            placeholder="Leave empty to use the default under app data"
            class="min-w-0 flex-1 rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2.5 font-mono text-xs"
          />
          <button
            type="button"
            class="shrink-0 rounded-xl border-2 border-lm-border px-5 py-3.5 text-base font-semibold hover:bg-lm-surface-hover"
            onclick={() => browseModelsDir()}
          >
            Browse
          </button>
        </div>
      </Card>

      <Card title="App data">
        <p class="mb-4 text-sm text-lm-muted">
          Your chats, library database, and reference files live here. To use the
          normal location, clear this field and save.
        </p>
        <div class="flex gap-2">
          <input
            type="text"
            bind:value={appDataDir}
            placeholder="Custom folder, or leave blank for default"
            class="min-w-0 flex-1 rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2.5 font-mono text-xs"
          />
          <button
            type="button"
            class="shrink-0 rounded-xl border-2 border-lm-border px-5 py-3.5 text-base font-semibold hover:bg-lm-surface-hover"
            onclick={() => browseAppDataDir()}
          >
            Browse
          </button>
        </div>
        {#if paths}
          <div
            class="mt-4 rounded-md border border-lm-border/80 bg-lm-bg/40 p-3 text-xs text-lm-muted"
          >
            <p>
              <span class="font-medium text-lm-text">Profile pointer file:</span>
              <span class="ml-1 font-mono" title={paths.builtinAppDataDir}
                >{paths.builtinAppDataDir}</span>
            </p>
            <p class="mt-2">
              <span class="font-medium text-lm-text">Currently in use:</span>
              <span class="ml-1 font-mono" title={paths.appDataDir}
                >{paths.appDataDir}</span>
            </p>
          </div>
        {/if}
      </Card>
    </div>

    <div class="mt-16 md:mt-24">
      <PageHeader title="Inference" />
    </div>

    <div class="space-y-6">
      <Card title="API Server">
        <p class="mb-4 text-sm text-lm-muted">
          Turn this on to let other apps connect to LocalMOD. It can keep running even after this
          window is closed.
        </p>

        {#if headlessMsg}
          <p class="mb-4 rounded-md border border-lm-border bg-lm-bg px-3 py-2 text-sm text-lm-text">
            {headlessMsg}
          </p>
        {/if}

        {#if headlessStatus?.lastError}
          <p class="mb-4 rounded-md border border-red-400/40 bg-red-950/20 px-3 py-2 text-sm text-red-200">
            {headlessStatus.lastError}
          </p>
        {/if}

        <div class="space-y-4">
          <div class="flex flex-wrap items-center justify-between gap-3 rounded-md border border-lm-border/80 bg-lm-bg/40 p-3 text-sm">
            <p>
              <span class="font-medium text-lm-text">Status:</span>
              <span class="ml-1 text-lm-muted">{headlessStatus?.running ? "On" : "Off"}</span>
            </p>
            <button
              type="button"
              class="rounded-xl border-2 border-lm-border px-4 py-2 text-sm font-semibold hover:bg-lm-surface-hover disabled:opacity-50"
              disabled={headlessBusy}
              onclick={() => void copyHeadlessBaseUrl()}
            >
              Copy URL
            </button>
          </div>

          <div>
            <label for="headless-url" class="mb-1 block text-sm font-medium text-lm-text">API URL</label>
            <input
              id="headless-url"
              type="text"
              readonly
              value={headlessStatus?.baseUrl ?? `http://${headlessHost}:${headlessPort}/v1`}
              class="w-full rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2 font-mono text-sm"
            />
          </div>

          <div class="grid gap-4 sm:grid-cols-[minmax(0,1fr)_8rem]">
            <div>
              <label for="headless-host" class="mb-1 block text-sm font-medium text-lm-text">
                Who can connect
              </label>
              <input
                id="headless-host"
                type="text"
                bind:value={headlessHost}
                disabled={headlessBusy || headlessStatus?.running}
                class="w-full rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2 font-mono text-sm disabled:opacity-60"
              />
              <p class="mt-1 text-xs text-lm-muted">
                127.0.0.1 = only this computer. 0.0.0.0 = other devices can connect.
              </p>
            </div>
            <div>
              <label for="headless-port" class="mb-1 block text-sm font-medium text-lm-text">Port</label>
              <input
                id="headless-port"
                type="number"
                bind:value={headlessPort}
                min="1"
                max="65535"
                disabled={headlessBusy || headlessStatus?.running}
                class="w-full rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2 text-sm tabular-nums disabled:opacity-60"
              />
            </div>
          </div>

          <div>
            <label for="headless-auth" class="mb-1 block text-sm font-medium text-lm-text">Auth</label>
            <select
              id="headless-auth"
              bind:value={headlessAuthMode}
              disabled={headlessBusy || headlessStatus?.running}
              class="w-full rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2 text-sm disabled:opacity-60"
            >
              <option value="none">No auth</option>
              <option value="bearer">Bearer API key</option>
            </select>
          </div>

          {#if headlessAuthMode === "bearer"}
            <div>
              <label for="headless-api-key" class="mb-1 block text-sm font-medium text-lm-text">API key</label>
              <div class="flex gap-2">
                <input
                  id="headless-api-key"
                  type="password"
                  bind:value={headlessApiKey}
                  disabled={headlessBusy || headlessStatus?.running}
                  class="min-w-0 flex-1 rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2 font-mono text-xs disabled:opacity-60"
                />
                <button
                  type="button"
                  class="shrink-0 rounded-xl border-2 border-lm-border px-4 py-2 text-sm font-semibold hover:bg-lm-surface-hover disabled:opacity-50"
                  disabled={headlessBusy || headlessStatus?.running}
                  onclick={() => (headlessApiKey = randomApiKey())}
                >
                  Generate
                </button>
                <button
                  type="button"
                  class="shrink-0 rounded-xl border-2 border-lm-border px-4 py-2 text-sm font-semibold hover:bg-lm-surface-hover disabled:opacity-50"
                  disabled={headlessBusy || headlessStatus?.running}
                  onclick={() => (headlessApiKey = "")}
                >
                  Clear
                </button>
              </div>
            </div>
          {/if}

          <details class="rounded-md border border-lm-border/80 bg-lm-bg/40 p-3 text-xs text-lm-muted">
            <summary class="cursor-pointer text-sm font-medium text-lm-text">Advanced</summary>
            <div class="mt-4 space-y-4">
              <div>
                <label for="headless-data-dir" class="mb-1 block text-sm font-medium text-lm-text">Data dir</label>
                <div class="flex gap-2">
                  <input
                    id="headless-data-dir"
                    type="text"
                    bind:value={headlessDataDir}
                    placeholder="Defaults to the configured app data folder"
                    disabled={headlessBusy || headlessStatus?.running}
                    class="min-w-0 flex-1 rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2.5 font-mono text-xs disabled:opacity-60"
                  />
                  <button
                    type="button"
                    class="shrink-0 rounded-xl border-2 border-lm-border px-4 py-2 text-sm font-semibold hover:bg-lm-surface-hover disabled:opacity-50"
                    disabled={headlessBusy || headlessStatus?.running}
                    onclick={() => void browseHeadlessDataDir()}
                  >
                    Browse
                  </button>
                </div>
              </div>

              <div>
                <label for="headless-models-dir" class="mb-1 block text-sm font-medium text-lm-text">
                  Models dir <span class="font-normal text-lm-muted">(optional)</span>
                </label>
                <div class="flex gap-2">
                  <input
                    id="headless-models-dir"
                    type="text"
                    bind:value={headlessModelsDir}
                    placeholder="Leave empty to use the server default"
                    disabled={headlessBusy || headlessStatus?.running}
                    class="min-w-0 flex-1 rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2.5 font-mono text-xs disabled:opacity-60"
                  />
                  <button
                    type="button"
                    class="shrink-0 rounded-xl border-2 border-lm-border px-4 py-2 text-sm font-semibold hover:bg-lm-surface-hover disabled:opacity-50"
                    disabled={headlessBusy || headlessStatus?.running}
                    onclick={() => void browseHeadlessModelsDir()}
                  >
                    Browse
                  </button>
                </div>
              </div>

              <div>
                <label for="headless-runtime-dir" class="mb-1 block text-sm font-medium text-lm-text">
                  Runtime dir <span class="font-normal text-lm-muted">(optional)</span>
                </label>
                <div class="flex gap-2">
                  <input
                    id="headless-runtime-dir"
                    type="text"
                    bind:value={headlessRuntimeDir}
                    placeholder="Leave empty to use the bundled runtime"
                    disabled={headlessBusy || headlessStatus?.running}
                    class="min-w-0 flex-1 rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2.5 font-mono text-xs disabled:opacity-60"
                  />
                  <button
                    type="button"
                    class="shrink-0 rounded-xl border-2 border-lm-border px-4 py-2 text-sm font-semibold hover:bg-lm-surface-hover disabled:opacity-50"
                    disabled={headlessBusy || headlessStatus?.running}
                    onclick={() => void browseHeadlessRuntimeDir()}
                  >
                    Browse
                  </button>
                </div>
              </div>

              <div class="rounded-md border border-lm-border/80 bg-lm-bg/40 p-3">
                {#if headlessStatus?.pid}
                  <p>
                    <span class="font-medium text-lm-text">PID:</span>
                    <span class="ml-1 font-mono">{headlessStatus.pid}</span>
                  </p>
                {/if}
                <p class={headlessStatus?.pid ? "mt-2" : ""}>
                  <span class="font-medium text-lm-text">Auth:</span>
                  <span class="ml-1">{headlessStatus?.authMode === "bearer" ? "Bearer API key" : "No auth"}</span>
                </p>
                <p class="mt-2">
                  <span class="font-medium text-lm-text">Data dir:</span>
                  <span class="ml-1 font-mono">{headlessStatus?.dataDir || headlessDataDir || appDataDir || "Default"}</span>
                </p>
                <p class="mt-2">
                  <span class="font-medium text-lm-text">Models dir:</span>
                  <span class="ml-1 font-mono">{headlessStatus?.modelsDir || headlessModelsDir || "Server default"}</span>
                </p>
                <p class="mt-2">
                  <span class="font-medium text-lm-text">Runtime dir:</span>
                  <span class="ml-1 font-mono">{headlessStatus?.runtimeDir || headlessRuntimeDir || "Bundled runtime"}</span>
                </p>
              </div>

              {#if headlessStatus?.command}
                <div class="rounded-md border border-lm-border/80 bg-lm-bg/40 p-3">
                  <p class="mb-2 font-medium text-lm-text">Command preview</p>
                  <pre class="whitespace-pre-wrap break-all font-mono">{headlessStatus.command}</pre>
                </div>
              {/if}
            </div>
          </details>

          <div class="flex flex-wrap justify-end gap-2">
            <button
              type="button"
              class="rounded-xl border-2 border-lm-border px-5 py-3 text-base font-semibold hover:bg-lm-surface-hover disabled:opacity-50"
              disabled={headlessBusy}
              onclick={() => void refreshHeadlessServer(true)}
            >
              Refresh
            </button>
            <button
              type="button"
              class="rounded-xl border-2 border-lm-border px-5 py-3 text-base font-semibold hover:bg-lm-surface-hover disabled:opacity-50"
              disabled={headlessBusy || !headlessStatus?.running}
              onclick={() => void stopStandaloneServer()}
            >
              Stop API server
            </button>
            <button
              type="button"
              class="rounded-xl border-2 border-lm-accent bg-lm-accent px-5 py-3 text-base font-bold text-lm-bg hover:bg-lm-accent-hover disabled:opacity-50"
              disabled={headlessBusy || headlessStatus?.running}
              onclick={() => void startStandaloneServer()}
            >
              {headlessBusy ? "Working..." : "Start API server"}
            </button>
          </div>
        </div>
      </Card>

      <Card title="Context & hardware">
        <div class="space-y-4">
          <div>
            <label for="set-n-ctx" class="mb-1 block text-sm font-medium text-lm-text">
              Context size (n_ctx, tokens)
            </label>
            <p class="mb-2 text-xs text-lm-muted">
              Max tokens the model can attend to (history + your message + references). If you see
              “exceeds the available context size”, raise this if you have RAM, or shorten the chat /
              fewer @ files.
            </p>
            <div class="flex flex-wrap items-center gap-2">
              <input
                id="set-n-ctx"
                type="number"
                bind:value={nCtx}
                min="512"
                max="262144"
                step="512"
                class="w-32 rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2 text-sm tabular-nums"
              />
              <span class="text-xs text-lm-muted">Quick:</span>
              {#each ctxPresets as [p, label] (p)}
                <button
                  type="button"
                  class="rounded-md border border-lm-border px-2 py-1 text-xs font-medium text-lm-muted hover:border-lm-accent/50 hover:text-lm-text"
                  onclick={() => (nCtx = p)}
                >
                  {label}
                </button>
              {/each}
            </div>
          </div>

          <div class="grid gap-4 sm:grid-cols-2">
            <div>
              <label for="set-n-threads" class="mb-1 block text-sm font-medium text-lm-text">
                CPU threads
              </label>
              <p class="mb-2 text-xs text-lm-muted">0 = automatic (recommended).</p>
              <input
                id="set-n-threads"
                type="number"
                bind:value={nThreads}
                min="0"
                max="256"
                class="w-full rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2 text-sm tabular-nums"
              />
            </div>
            <div>
              <label for="set-ngl" class="mb-1 block text-sm font-medium text-lm-text">
                GPU layers (-ngl)
              </label>
              <p class="mb-2 text-xs text-lm-muted">0 = CPU only. Set &gt;0 for CUDA/Metal builds.</p>
              <input
                id="set-ngl"
                type="number"
                bind:value={nGpuLayers}
                min="0"
                max="999"
                class="w-full rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2 text-sm tabular-nums"
              />
            </div>
          </div>
        </div>
      </Card>

      <Card title="Generation">
        <div class="grid gap-4 sm:grid-cols-2">
          <div>
            <label for="set-max-tok" class="mb-1 block text-sm font-medium text-lm-text">
              Max reply tokens
            </label>
            <p class="mb-2 text-xs text-lm-muted">Caps each assistant message length.</p>
            <input
              id="set-max-tok"
              type="number"
              bind:value={maxTokens}
              min="16"
              max="32768"
              class="w-full rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2 text-sm tabular-nums"
            />
          </div>
          <div>
            <label for="set-seed" class="mb-1 block text-sm font-medium text-lm-text">Seed</label>
            <p class="mb-2 text-xs text-lm-muted">Sampling seed (determinism hint).</p>
            <input
              id="set-seed"
              type="number"
              bind:value={seed}
              min="0"
              max="4294967295"
              class="w-full rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2 text-sm tabular-nums"
            />
          </div>
          <div>
            <label for="set-temp" class="mb-1 block text-sm font-medium text-lm-text"
              >Temperature</label
            >
            <input
              id="set-temp"
              type="number"
              bind:value={temperature}
              min="0"
              max="2"
              step="0.05"
              class="w-full rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2 text-sm tabular-nums"
            />
          </div>
          <div>
            <label for="set-topp" class="mb-1 block text-sm font-medium text-lm-text">Top P</label>
            <input
              id="set-topp"
              type="number"
              bind:value={topP}
              min="0.05"
              max="1"
              step="0.05"
              class="w-full rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2 text-sm tabular-nums"
            />
          </div>
        </div>
      </Card>
    </div>

    <div class="mt-8 flex justify-end">
      <button
        type="button"
        class="rounded-xl border-2 border-lm-accent bg-lm-accent px-10 py-4 text-base font-bold text-lm-bg hover:bg-lm-accent-hover"
        onclick={() => save()}
      >
        Save settings
      </button>
    </div>
  </div>
</div>
