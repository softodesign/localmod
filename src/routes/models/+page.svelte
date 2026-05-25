<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import * as api from "$lib/tauri-bridge";
  import { isLoadableModel } from "$lib/model-utils";

  let query = $state("");
  let models = $state<api.ModelDto[]>([]);
  let loaded = $state<api.LoadedDto | null>(null);
  let err = $state<string | null>(null);

  let knowledgeModal = $state<api.ModelKnowledgeDto | null>(null);

  let hfRepo = $state("");
  let hfFiles = $state<api.HfWeightFileDto[]>([]);
  let hfSelected = $state("");
  let hfListing = $state(false);
  let hfStartBusy = $state(false);

  let cloudModal = $state(false);
  let cloudTab = $state<"openai" | "anthropic" | "openrouter" | "custom">("openai");
  let cloudKeyOpenai = $state("");
  let cloudModelOpenai = $state("");
  let cloudKeyAnthropic = $state("");
  let cloudModelAnthropic = $state("");
  let cloudKeyOpenrouter = $state("");
  let cloudModelOpenrouter = $state("");
  let cloudKeyCustom = $state("");
  let cloudModelCustom = $state("");
  let cloudBaseUrlCustom = $state("");
  let cloudHasKeyOpenai = $state(false);
  let cloudHasKeyAnthropic = $state(false);
  let cloudHasKeyOpenrouter = $state(false);
  let cloudHasKeyCustom = $state(false);
  let cloudImageGenOpenai = $state(false);
  let cloudImageModelOpenai = $state("");
  let cloudImageGenOpenrouter = $state(false);
  let cloudImageModelOpenrouter = $state("");
  let cloudImageGenCustom = $state(false);
  let cloudImageModelCustom = $state("");
  let cloudSaveBusy = $state(false);

  let editModalOpen = $state(false);
  let editModel = $state<api.ModelDto | null>(null);
  let editName = $state("");
  let editPath = $state("");
  let editSaveBusy = $state(false);

  let downloadToast = $state<{
    variant: "success" | "error";
    message: string;
  } | null>(null);
  let downloadToastTimer: ReturnType<typeof setTimeout> | undefined;

  function showDownloadToast(
    variant: "success" | "error",
    message: string,
  ) {
    if (downloadToastTimer !== undefined) clearTimeout(downloadToastTimer);
    downloadToast = { variant, message };
    downloadToastTimer = setTimeout(
      () => {
        downloadToast = null;
        downloadToastTimer = undefined;
      },
      variant === "success" ? 4500 : 9000,
    );
  }

  async function refresh() {
    models = await api.listModels();
    loaded = await api.getLoadedLlm();
  }

  async function openCloudModal() {
    err = null;
    try {
      const list = await api.getCloudProviderConfigs();
      cloudKeyOpenai = "";
      cloudKeyAnthropic = "";
      cloudKeyOpenrouter = "";
      cloudKeyCustom = "";
      for (const p of list) {
        if (p.id === "openai") {
          cloudModelOpenai = p.model;
          cloudHasKeyOpenai = p.hasApiKey;
          cloudImageGenOpenai = p.imageGenerationEnabled ?? false;
          cloudImageModelOpenai = p.imageModel ?? "";
        } else if (p.id === "anthropic") {
          cloudModelAnthropic = p.model;
          cloudHasKeyAnthropic = p.hasApiKey;
        } else if (p.id === "openrouter") {
          cloudModelOpenrouter = p.model;
          cloudHasKeyOpenrouter = p.hasApiKey;
          cloudImageGenOpenrouter = p.imageGenerationEnabled ?? false;
          cloudImageModelOpenrouter = p.imageModel ?? "";
        } else if (p.id === "custom") {
          cloudModelCustom = p.model;
          cloudBaseUrlCustom = p.baseUrl ?? "";
          cloudHasKeyCustom = p.hasApiKey;
          cloudImageGenCustom = p.imageGenerationEnabled ?? false;
          cloudImageModelCustom = p.imageModel ?? "";
        }
      }
      cloudModal = true;
    } catch (e) {
      err = String(e);
    }
  }

  function closeCloudModal() {
    cloudModal = false;
  }

  async function saveCloudTab() {
    cloudSaveBusy = true;
    err = null;
    try {
      if (cloudTab === "openai") {
        await api.setCloudProviderConfig(
          "openai",
          cloudKeyOpenai,
          cloudModelOpenai,
          null,
          cloudImageGenOpenai,
          cloudImageModelOpenai,
        );
      } else if (cloudTab === "anthropic") {
        await api.setCloudProviderConfig("anthropic", cloudKeyAnthropic, cloudModelAnthropic);
      } else if (cloudTab === "openrouter") {
        await api.setCloudProviderConfig(
          "openrouter",
          cloudKeyOpenrouter,
          cloudModelOpenrouter,
          null,
          cloudImageGenOpenrouter,
          cloudImageModelOpenrouter,
        );
      } else {
        await api.setCloudProviderConfig(
          "custom",
          cloudKeyCustom,
          cloudModelCustom,
          cloudBaseUrlCustom,
          cloudImageGenCustom,
          cloudImageModelCustom,
        );
      }
      await refresh();
      closeCloudModal();
    } catch (e) {
      err = String(e);
    } finally {
      cloudSaveBusy = false;
    }
  }

  async function clearCloudTab() {
    cloudSaveBusy = true;
    err = null;
    try {
      const prov = cloudTab;
      await api.setCloudProviderConfig(prov, "", "");
      if (prov === "openai") {
        cloudModelOpenai = "";
        cloudHasKeyOpenai = false;
      } else if (prov === "anthropic") {
        cloudModelAnthropic = "";
        cloudHasKeyAnthropic = false;
      } else if (prov === "openrouter") {
        cloudModelOpenrouter = "";
        cloudHasKeyOpenrouter = false;
      } else {
        cloudModelCustom = "";
        cloudBaseUrlCustom = "";
        cloudHasKeyCustom = false;
      }
      await refresh();
      closeCloudModal();
    } catch (e) {
      err = String(e);
    } finally {
      cloudSaveBusy = false;
    }
  }

  onMount(() => {
    refresh();
    return () => {
      if (downloadToastTimer !== undefined) clearTimeout(downloadToastTimer);
    };
  });

  const filtered = $derived(
    models.filter((m) => m.name.toLowerCase().includes(query.toLowerCase())),
  );

  function closeKnowledgeModal() {
    knowledgeModal = null;
  }

  async function importFromDevice() {
    err = null;
    try {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [
          { name: "Weights", extensions: ["gguf", "safetensors"] },
        ],
      });
      if (path === null) return;
      const p = typeof path === "string" ? path : path[0];
      const dto = await api.registerModel(p);
      await refresh();
      knowledgeModal = await api.getModelKnowledge(dto.path);
    } catch (e) {
      err = String(e);
    }
  }

  async function downloadHfAuto() {
    err = null;
    const r = hfRepo.trim();
    if (!r) {
      showDownloadToast("error", "Repo ID or URL required.");
      return;
    }
    hfStartBusy = true;
    try {
      await api.hfDownloadStartAuto(r);
      showDownloadToast(
        "success",
        "Download started. Open Downloads to track progress.",
      );
    } catch (e) {
      showDownloadToast("error", `Could not start download: ${String(e)}`);
    } finally {
      hfStartBusy = false;
    }
  }

  async function listHfFiles() {
    err = null;
    hfFiles = [];
    hfSelected = "";
    const r = hfRepo.trim();
    if (!r) {
      err = "Repo ID or URL required.";
      return;
    }
    hfListing = true;
    try {
      hfFiles = await api.listHuggingfaceGgufFiles(r, null);
      if (hfFiles.length === 0) {
        err = "No weight files in repo.";
      }
    } catch (e) {
      err = String(e);
    } finally {
      hfListing = false;
    }
  }

  async function downloadHfSelected() {
    err = null;
    if (!hfSelected) {
      showDownloadToast("error", "Select a file first.");
      return;
    }
    hfStartBusy = true;
    try {
      await api.hfDownloadStartManual(hfRepo.trim(), hfSelected, null);
      showDownloadToast(
        "success",
        "Download started. Open Downloads to track progress.",
      );
    } catch (e) {
      showDownloadToast("error", `Could not start download: ${String(e)}`);
    } finally {
      hfStartBusy = false;
    }
  }

  function formatHfSize(bytes: number | null): string {
    if (bytes == null) return "";
    const gb = bytes / 1024 ** 3;
    if (gb >= 1) return ` · ${gb.toFixed(2)} GB`;
    const mb = bytes / 1024 ** 2;
    return ` · ${mb.toFixed(0)} MB`;
  }

  let loadBusy = $state(false);

  async function load(id: string) {
    err = null;
    loadBusy = true;
    try {
      loaded = await api.loadLlm(id);
    } catch (e) {
      err = String(e);
    } finally {
      loadBusy = false;
    }
  }

  async function remove(id: string) {
    err = null;
    try {
      await api.deleteModel(id);
      await refresh();
    } catch (e) {
      err = String(e);
    }
  }

  function sizeGb(m: api.ModelDto): string {
    if (m.sizeBytes == null) return "—";
    return (m.sizeBytes / 1024 ** 3).toFixed(2);
  }

  function canLoad(m: api.ModelDto): boolean {
    return isLoadableModel(m);
  }

  function cloudTabForModelId(id: string): typeof cloudTab | null {
    if (id === "lm-cloud-openai") return "openai";
    if (id === "lm-cloud-anthropic") return "anthropic";
    if (id === "lm-cloud-openrouter") return "openrouter";
    if (id === "lm-cloud-custom") return "custom";
    return null;
  }

  function openEditModel(m: api.ModelDto) {
    err = null;
    if (m.weightsFormat === "cloud") {
      const tab = cloudTabForModelId(m.id);
      if (tab) {
        cloudTab = tab;
        void openCloudModal();
      } else {
        err = "Edit this cloud model via Set up cloud models.";
      }
      return;
    }
    editModel = m;
    editName = m.name;
    editPath = m.path;
    editModalOpen = true;
  }

  function closeEditModal() {
    editModalOpen = false;
    editModel = null;
    editName = "";
    editPath = "";
    editSaveBusy = false;
  }

  async function browseEditPath() {
    if (!editModel) return;
    const ext = editModel.weightsFormat === "safetensors" ? "safetensors" : "gguf";
    const path = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "Weights", extensions: [ext] }],
    });
    if (path === null) return;
    editPath = typeof path === "string" ? path : path[0];
  }

  async function saveEditModel() {
    if (!editModel || editSaveBusy) return;
    const name = editName.trim();
    if (!name) {
      err = "Display name is required.";
      return;
    }
    editSaveBusy = true;
    err = null;
    try {
      const patch: { name: string; path?: string } = { name };
      if (editPath.trim() !== editModel.path) {
        patch.path = editPath.trim();
      }
      await api.updateModel(editModel.id, patch);
      closeEditModal();
      await refresh();
    } catch (e) {
      err = String(e);
    } finally {
      editSaveBusy = false;
    }
  }
</script>

<div class="flex-1 overflow-y-auto overscroll-y-contain">
  <div class="mx-auto max-w-7xl px-4 py-7 sm:px-8 md:py-10 lg:px-12">
    <PageHeader title="Models"></PageHeader>

    {#if loadBusy}
      <p class="mb-4 rounded-md border border-lm-border bg-lm-elevated px-3 py-2 text-sm text-lm-muted">
        Loading model…
      </p>
    {:else if err}
      <p class="mb-4 rounded-md border border-red-900/50 bg-red-950/40 px-3 py-2 text-sm text-red-200">
        {err}
      </p>
    {/if}

    <div class="mb-8 grid gap-4 md:grid-cols-2 md:gap-6">
      <Card title="Disk">
        <button
          type="button"
          class="min-h-14 w-full rounded-xl border-2 border-lm-border bg-lm-elevated px-5 py-3 text-base font-semibold hover:bg-lm-surface-hover sm:w-auto"
          onclick={() => importFromDevice()}
        >
          Choose .gguf / .safetensors
        </button>
        <p class="mt-3 text-xs leading-relaxed text-lm-muted">
          Chat and <strong class="font-medium text-lm-text">Load</strong> only work with
          <strong class="font-medium text-lm-text">.gguf</strong> (llama.cpp). Safetensors can be
          added to the library but are not runnable here — use a GGUF build of the model.
        </p>
      </Card>

      <Card title="Hugging Face">
        <p class="mb-2 text-xs text-lm-muted">
          <strong class="font-medium text-lm-text">GGUF only.</strong> Safetensors from the repo
          cannot be loaded for chat—only <code class="font-mono text-[0.65rem]">.gguf</code> weight files
          work here.
        </p>
        <div class="flex flex-col gap-2 sm:flex-row sm:items-stretch">
          <input
            bind:value={hfRepo}
            type="text"
            placeholder="org/model or URL"
            class="min-h-14 flex-1 rounded-xl border-2 border-lm-border bg-lm-bg px-4 py-3 text-base"
            disabled={hfStartBusy}
          />
          <button
            type="button"
            class="min-h-14 shrink-0 rounded-xl border-2 border-lm-accent bg-lm-accent px-5 py-3 text-base font-semibold text-lm-bg hover:bg-lm-accent-hover disabled:opacity-50"
            disabled={hfStartBusy || !hfRepo.trim()}
            onclick={() => downloadHfAuto()}
          >
            {hfStartBusy ? "…" : "Download"}
          </button>
        </div>
        <p class="mt-2 text-xs text-lm-muted">
          Transfers continue in the background. Open <a href="/downloads" class="font-semibold text-lm-accent hover:underline">Downloads</a> for progress, pause, and cancel.
        </p>
        <details class="mt-3 text-sm">
          <summary class="cursor-pointer font-medium text-lm-text">Pick file</summary>
          <div class="mt-2 flex flex-wrap gap-2">
            <button
              type="button"
              class="rounded border border-lm-border px-3 py-1.5 text-xs font-semibold hover:bg-lm-surface-hover disabled:opacity-50"
              disabled={hfListing || hfStartBusy || !hfRepo.trim()}
              onclick={() => listHfFiles()}
            >
              {hfListing ? "…" : "List"}
            </button>
          </div>
          {#if hfFiles.length > 0}
            <select
              bind:value={hfSelected}
              class="mt-2 w-full rounded-md border-2 border-lm-border bg-lm-bg px-2 py-2 text-xs"
              disabled={hfStartBusy}
            >
              <option value="">—</option>
              {#each hfFiles as f (f.path)}
                <option value={f.path}
                  >{f.kind} · {f.path}{formatHfSize(f.size)}</option>
              {/each}
            </select>
            <button
              type="button"
              class="mt-2 rounded-md border border-lm-border bg-lm-elevated px-3 py-2 text-xs font-semibold disabled:opacity-50"
              disabled={hfStartBusy || !hfSelected}
              onclick={() => downloadHfSelected()}
            >
              Download selection
            </button>
          {/if}
        </details>
      </Card>
    </div>

    <div class="mb-8">
      <Card title="Cloud providers">
        <p class="mb-3 text-sm text-lm-muted">
          Connect <strong class="text-lm-text">OpenAI</strong>,
          <strong class="text-lm-text">Anthropic</strong>,
          <strong class="text-lm-text">OpenRouter</strong>, or a
          <strong class="text-lm-text">Custom</strong> OpenAI-compatible API (base URL + key + model id).
          Keys stay in your local database.
        </p>
        <button
          type="button"
          class="min-h-14 rounded-xl border-2 border-lm-accent bg-lm-accent/15 px-5 py-3 text-base font-bold text-lm-accent hover:bg-lm-accent/25"
          onclick={() => void openCloudModal()}
        >
          Set up cloud models
        </button>
      </Card>
    </div>

    {#if loaded}
      <p class="mb-4 text-sm text-lm-muted">
        Loaded: <span class="font-semibold text-lm-text">{loaded.name}</span>
        <button
          type="button"
          class="ml-2 font-semibold text-lm-accent hover:underline"
          onclick={async () => {
            await api.unloadLlm();
            loaded = null;
          }}
        >
          Unload
        </button>
      </p>
    {/if}

    <input
      type="search"
      bind:value={query}
      placeholder="Filter models…"
      class="mb-2 w-full max-w-md rounded-xl border-2 border-lm-border bg-lm-surface px-4 py-3 text-base"
    />

    <div class="mb-4 flex flex-wrap items-baseline justify-between gap-2">
      <h2 class="text-lg font-bold text-lm-text">Your models</h2>
      <span class="text-sm text-lm-muted tabular-nums"
        >{filtered.length}{models.length !== filtered.length
          ? ` of ${models.length}`
          : ""}</span
      >
    </div>

    <div class="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
      {#each filtered as m (m.id)}
        <Card>
          <div class="flex items-start justify-between gap-2">
            <h3 class="break-words text-base font-semibold">{m.name}</h3>
            {#if loaded?.id === m.id}
              <Badge variant="accent">on</Badge>
            {:else}
              <Badge variant="muted">{m.weightsFormat}</Badge>
            {/if}
          </div>
          <p class="mt-1 text-xs text-lm-muted">
            {sizeGb(m)} GB
            {#if m.shardTotal != null && m.shardTotal > 1}
              · shard 1/{m.shardTotal}
            {/if}
          </p>
          <p class="mt-2 break-all font-mono text-[0.65rem] text-lm-muted">{m.path}</p>
          <div class="mt-3 flex flex-wrap gap-2 border-t border-lm-border pt-3">
            <button
              type="button"
              class="min-h-12 rounded-xl border-2 border-lm-accent bg-lm-accent px-4 py-2 text-sm font-semibold text-lm-bg hover:bg-lm-accent-hover disabled:opacity-40"
              disabled={loadBusy || !canLoad(m)}
              title={!canLoad(m)
                ? "This format cannot be loaded here."
                : m.weightsFormat === "cloud"
                  ? "Select for Chat — answers come from the provider’s API."
                  : "Load GGUF into llama-server."}
              onclick={() => load(m.id)}
            >
              Load
            </button>
            <button
              type="button"
              class="min-h-12 rounded-xl border-2 border-lm-border px-4 py-2 text-sm font-semibold hover:bg-lm-surface-hover"
              title={m.weightsFormat === "cloud"
                ? "Edit cloud provider settings"
                : "Edit display name, type, or file path"}
              onclick={() => openEditModel(m)}
            >
              Edit
            </button>
            <button
              type="button"
              class="min-h-12 rounded-xl border-2 border-lm-border px-4 py-2 text-sm font-semibold hover:bg-lm-surface-hover"
              title="Deletes the files from disk and removes this entry."
              onclick={() => remove(m.id)}
            >
              Delete
            </button>
          </div>
        </Card>
      {/each}
    </div>

    {#if filtered.length === 0}
      <p class="py-10 text-center text-sm text-lm-muted">
        {models.length === 0 ? "Nothing registered." : "No matches."}
      </p>
    {/if}
  </div>
</div>

{#if cloudModal}
  <div
    class="fixed inset-0 z-[60] flex items-end justify-center bg-black/55 p-3 sm:items-center"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeCloudModal();
    }}
  >
    <div
      class="max-h-[90vh] w-full max-w-lg overflow-hidden rounded-2xl border border-lm-border bg-lm-elevated"
      role="dialog"
      tabindex="-1"
      aria-modal="true"
      aria-labelledby="cloud-modal-title"
      onkeydown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          closeCloudModal();
        }
      }}
    >
      <div class="border-b border-lm-border px-4 py-3">
        <h2 id="cloud-modal-title" class="text-base font-bold">Cloud providers</h2>
        <p class="mt-1 text-xs text-lm-muted">
          API keys are stored only in your local database. Use the provider’s exact model id strings.
        </p>
      </div>
      <div class="grid grid-cols-2 gap-1 border-b border-lm-border px-2 py-2 sm:grid-cols-4">
        <button
          type="button"
          class="rounded-lg px-2 py-2.5 text-sm font-semibold {cloudTab === 'openai'
            ? 'bg-lm-accent/20 text-lm-accent'
            : 'text-lm-muted hover:bg-lm-surface-hover'}"
          onclick={() => {
            cloudTab = "openai";
          }}
        >
          OpenAI
        </button>
        <button
          type="button"
          class="rounded-lg px-2 py-2.5 text-sm font-semibold {cloudTab === 'anthropic'
            ? 'bg-lm-accent/20 text-lm-accent'
            : 'text-lm-muted hover:bg-lm-surface-hover'}"
          onclick={() => {
            cloudTab = "anthropic";
          }}
        >
          Anthropic
        </button>
        <button
          type="button"
          class="rounded-lg px-2 py-2.5 text-sm font-semibold {cloudTab === 'openrouter'
            ? 'bg-lm-accent/20 text-lm-accent'
            : 'text-lm-muted hover:bg-lm-surface-hover'}"
          onclick={() => {
            cloudTab = "openrouter";
          }}
        >
          OpenRouter
        </button>
        <button
          type="button"
          class="rounded-lg px-2 py-2.5 text-sm font-semibold {cloudTab === 'custom'
            ? 'bg-lm-accent/20 text-lm-accent'
            : 'text-lm-muted hover:bg-lm-surface-hover'}"
          onclick={() => {
            cloudTab = "custom";
          }}
        >
          Custom
        </button>
      </div>
      <div class="max-h-[min(60vh,28rem)] space-y-3 overflow-y-auto px-4 py-4">
        {#if cloudTab === "openai"}
          <div>
            <label class="block text-sm font-semibold text-lm-muted" for="ckey-o">API key</label>
            <input
              id="ckey-o"
              type="password"
              autocomplete="off"
              bind:value={cloudKeyOpenai}
              placeholder={cloudHasKeyOpenai ? "Leave blank to keep current key" : "sk-..."}
              class="mt-1 w-full rounded-xl border-2 border-lm-border bg-lm-bg px-4 py-2.5 text-base"
            />
          </div>
          <div>
            <label class="block text-sm font-semibold text-lm-muted" for="cmodel-o">Model id</label>
            <input
              id="cmodel-o"
              type="text"
              bind:value={cloudModelOpenai}
              placeholder="gpt-4o"
              class="mt-1 w-full rounded-xl border-2 border-lm-border bg-lm-bg px-4 py-2.5 font-mono text-base"
            />
          </div>
          <div class="rounded-xl border border-lm-border bg-lm-bg/40 p-3">
            <label class="flex cursor-pointer items-center gap-2 text-sm font-semibold text-lm-text">
              <input type="checkbox" bind:checked={cloudImageGenOpenai} class="size-4 rounded" />
              Allow image generation
            </label>
            {#if cloudImageGenOpenai}
              <label class="mt-2 block text-sm font-semibold text-lm-muted" for="cimg-o"
                >Image model id</label
              >
              <input
                id="cimg-o"
                type="text"
                bind:value={cloudImageModelOpenai}
                placeholder="dall-e-3"
                class="mt-1 w-full rounded-xl border-2 border-lm-border bg-lm-bg px-4 py-2.5 font-mono text-base"
              />
            {/if}
          </div>
        {:else if cloudTab === "anthropic"}
          <div>
            <label class="block text-sm font-semibold text-lm-muted" for="ckey-a">API key</label>
            <input
              id="ckey-a"
              type="password"
              autocomplete="off"
              bind:value={cloudKeyAnthropic}
              placeholder={cloudHasKeyAnthropic ? "Leave blank to keep current key" : "sk-ant-..."}
              class="mt-1 w-full rounded-xl border-2 border-lm-border bg-lm-bg px-4 py-2.5 text-base"
            />
          </div>
          <div>
            <label class="block text-sm font-semibold text-lm-muted" for="cmodel-a">Model id</label>
            <input
              id="cmodel-a"
              type="text"
              bind:value={cloudModelAnthropic}
              placeholder="claude-sonnet-4-20250514"
              class="mt-1 w-full rounded-xl border-2 border-lm-border bg-lm-bg px-4 py-2.5 font-mono text-base"
            />
          </div>
        {:else if cloudTab === "openrouter"}
          <div>
            <label class="block text-sm font-semibold text-lm-muted" for="ckey-r">API key</label>
            <input
              id="ckey-r"
              type="password"
              autocomplete="off"
              bind:value={cloudKeyOpenrouter}
              placeholder={cloudHasKeyOpenrouter ? "Leave blank to keep current key" : "sk-or-..."}
              class="mt-1 w-full rounded-xl border-2 border-lm-border bg-lm-bg px-4 py-2.5 text-base"
            />
          </div>
          <div>
            <label class="block text-sm font-semibold text-lm-muted" for="cmodel-r">Model id</label>
            <input
              id="cmodel-r"
              type="text"
              bind:value={cloudModelOpenrouter}
              placeholder="openai/gpt-4o"
              class="mt-1 w-full rounded-xl border-2 border-lm-border bg-lm-bg px-4 py-2.5 font-mono text-base"
            />
          </div>
          <div class="rounded-xl border border-lm-border bg-lm-bg/40 p-3">
            <label class="flex cursor-pointer items-center gap-2 text-sm font-semibold text-lm-text">
              <input type="checkbox" bind:checked={cloudImageGenOpenrouter} class="size-4 rounded" />
              Allow image generation
            </label>
            {#if cloudImageGenOpenrouter}
              <label class="mt-2 block text-sm font-semibold text-lm-muted" for="cimg-r"
                >Image model id</label
              >
              <input
                id="cimg-r"
                type="text"
                bind:value={cloudImageModelOpenrouter}
                placeholder="openai/dall-e-3"
                class="mt-1 w-full rounded-xl border-2 border-lm-border bg-lm-bg px-4 py-2.5 font-mono text-base"
              />
            {/if}
          </div>
        {:else}
          <p class="text-xs leading-relaxed text-lm-muted">
            For any OpenAI-compatible API (LM Studio, Ollama, Groq, local gateways, etc.). We POST to
            <code class="font-mono text-[0.65rem]">{'{baseUrl}'}/chat/completions</code>
            unless you include the full path.
          </p>
          <div>
            <label class="block text-sm font-semibold text-lm-muted" for="cbase-u">Base URL</label>
            <input
              id="cbase-u"
              type="url"
              bind:value={cloudBaseUrlCustom}
              placeholder="http://127.0.0.1:1234/v1"
              class="mt-1 w-full rounded-xl border-2 border-lm-border bg-lm-bg px-4 py-2.5 font-mono text-base"
            />
          </div>
          <div>
            <label class="block text-sm font-semibold text-lm-muted" for="ckey-u">API key</label>
            <input
              id="ckey-u"
              type="password"
              autocomplete="off"
              bind:value={cloudKeyCustom}
              placeholder={cloudHasKeyCustom ? "Leave blank to keep current key" : "Optional for some local servers"}
              class="mt-1 w-full rounded-xl border-2 border-lm-border bg-lm-bg px-4 py-2.5 text-base"
            />
          </div>
          <div>
            <label class="block text-sm font-semibold text-lm-muted" for="cmodel-u">Model id</label>
            <input
              id="cmodel-u"
              type="text"
              bind:value={cloudModelCustom}
              placeholder="llama-3.1-8b-instruct"
              class="mt-1 w-full rounded-xl border-2 border-lm-border bg-lm-bg px-4 py-2.5 font-mono text-base"
            />
          </div>
          <div class="rounded-xl border border-lm-border bg-lm-bg/40 p-3">
            <label class="flex cursor-pointer items-center gap-2 text-sm font-semibold text-lm-text">
              <input type="checkbox" bind:checked={cloudImageGenCustom} class="size-4 rounded" />
              Allow image generation
            </label>
            {#if cloudImageGenCustom}
              <label class="mt-2 block text-sm font-semibold text-lm-muted" for="cimg-u"
                >Image model id</label
              >
              <input
                id="cimg-u"
                type="text"
                bind:value={cloudImageModelCustom}
                placeholder="dall-e-3"
                class="mt-1 w-full rounded-xl border-2 border-lm-border bg-lm-bg px-4 py-2.5 font-mono text-base"
              />
              <p class="mt-1 text-xs text-lm-muted">
                POSTs to <code class="font-mono">{'{baseUrl}'}/images/generations</code>
              </p>
            {/if}
          </div>
        {/if}
      </div>
      <div class="flex flex-wrap items-center gap-2 border-t border-lm-border px-4 py-3">
        <button
          type="button"
          class="rounded-xl border-2 border-lm-border px-4 py-3 text-base font-semibold hover:bg-lm-surface-hover"
          onclick={() => closeCloudModal()}
        >
          Cancel
        </button>
        <button
          type="button"
          class="rounded-xl border-2 border-red-900/50 px-4 py-3 text-base font-semibold text-red-200 hover:bg-red-950/35 disabled:opacity-50"
          disabled={cloudSaveBusy}
          onclick={() => void clearCloudTab()}
        >
          Clear
        </button>
        <button
          type="button"
          class="ml-auto rounded-xl border-2 border-lm-accent bg-lm-accent px-5 py-3 text-base font-bold text-lm-bg hover:bg-lm-accent-hover disabled:opacity-50"
          disabled={cloudSaveBusy}
          onclick={() => void saveCloudTab()}
        >
          {cloudSaveBusy ? "Saving…" : "Save"}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if editModalOpen && editModel}
  <div
    class="fixed inset-0 z-[70] flex items-center justify-center bg-black/55 p-4"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget && !editSaveBusy) closeEditModal();
    }}
  >
    <div
      class="max-h-[min(90vh,40rem)] w-full max-w-lg overflow-hidden rounded-2xl border border-lm-border bg-lm-surface"
      role="dialog"
      tabindex="-1"
      aria-modal="true"
      aria-labelledby="edit-model-title"
      onkeydown={(e) => {
        if (e.key === "Escape" && !editSaveBusy) {
          e.preventDefault();
          closeEditModal();
        }
      }}
    >
      <div class="border-b border-lm-border px-5 py-4">
        <h2 id="edit-model-title" class="text-lg font-bold text-lm-text">Edit model</h2>
        <p class="mt-1 text-sm text-lm-muted">
          {editModel.weightsFormat.toUpperCase()}
          {#if editModel.shardTotal != null && editModel.shardTotal > 1}
            · shard {editModel.shardIndex ?? 1}/{editModel.shardTotal}
          {/if}
        </p>
      </div>
      <div class="max-h-[min(55vh,24rem)] space-y-4 overflow-y-auto px-5 py-4">
        <div>
          <label class="block text-sm font-semibold text-lm-muted" for="edit-model-name"
            >Display name</label
          >
          <input
            id="edit-model-name"
            type="text"
            bind:value={editName}
            disabled={editSaveBusy}
            class="mt-1 w-full rounded-xl border border-lm-border bg-lm-bg px-4 py-2.5 text-base"
          />
        </div>
        <div>
          <label class="block text-sm font-semibold text-lm-muted" for="edit-model-path"
            >File path</label
          >
          <div class="mt-1 flex gap-2">
            <input
              id="edit-model-path"
              type="text"
              bind:value={editPath}
              disabled={editSaveBusy}
              class="min-w-0 flex-1 rounded-xl border border-lm-border bg-lm-bg px-3 py-2.5 font-mono text-xs"
            />
            <button
              type="button"
              class="shrink-0 rounded-xl border border-lm-border px-3 py-2 text-sm font-semibold hover:bg-lm-surface-hover disabled:opacity-50"
              disabled={editSaveBusy}
              onclick={() => void browseEditPath()}
            >
              Browse
            </button>
          </div>
          <p class="mt-1.5 text-xs text-lm-muted">
            Change this if you moved the file. The path must still point at the same weights format.
          </p>
        </div>
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
          disabled={editSaveBusy || !editName.trim()}
          onclick={() => void saveEditModel()}
        >
          {editSaveBusy ? "Saving…" : "Save"}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if downloadToast}
  <div
    class="pointer-events-none fixed bottom-6 left-1/2 z-[100] w-[min(28rem,calc(100vw-1.5rem))] -translate-x-1/2 px-3"
    role="status"
    aria-live="polite"
  >
    <div
      class="pointer-events-auto rounded-xl border px-5 py-4 text-base {downloadToast.variant ===
      'success'
        ? 'border-emerald-800/60 bg-emerald-950/90 text-emerald-100'
        : 'border-red-900/50 bg-red-950/90 text-red-100'}"
    >
      <p class="font-semibold">
        {downloadToast.variant === "success"
          ? "Download started"
          : "Download failed"}
      </p>
      <p class="mt-1 text-xs opacity-90">{downloadToast.message}</p>
    </div>
  </div>
{/if}

{#if knowledgeModal}
  <div
    class="fixed inset-0 z-50 flex items-end justify-center bg-black/55 p-3 sm:items-center"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeKnowledgeModal();
    }}
  >
    <div
      class="max-h-[85vh] w-full max-w-md overflow-y-auto rounded-2xl border border-lm-border bg-lm-elevated"
      role="dialog"
      aria-modal="true"
      aria-labelledby="km-title"
    >
      <div class="border-b border-lm-border px-4 py-3">
        <h2 id="km-title" class="text-base font-bold">{knowledgeModal.displayName}</h2>
        <p class="mt-1 text-xs text-lm-muted">
          {#if knowledgeModal.supportsImages}
            Images: yes (mmproj)
          {:else}
            Images: no
          {/if}
        </p>
      </div>
      <div class="space-y-2 px-5 py-4 text-base text-lm-text">
        <p>{knowledgeModal.summary}</p>
        <p class="text-xs text-lm-muted">{knowledgeModal.visionExplanation}</p>
        <ul class="list-disc space-y-1 pl-4 text-xs">
          {#each knowledgeModal.capabilities as line (line)}
            <li>{line}</li>
          {/each}
        </ul>
      </div>
      <div class="border-t border-lm-border px-4 py-3">
        <button
          type="button"
          class="w-full rounded-xl border-2 border-lm-accent bg-lm-accent py-3 text-base font-semibold text-lm-bg"
          onclick={() => closeKnowledgeModal()}
        >
          Close
        </button>
      </div>
    </div>
  </div>
{/if}
