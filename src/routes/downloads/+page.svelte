<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import ProgressBar from "$lib/components/ui/ProgressBar.svelte";
  import * as api from "$lib/tauri-bridge";

  let jobs = $state<api.HfDownloadJobDto[]>([]);
  let err = $state<string | null>(null);
  let interval: ReturnType<typeof setInterval> | null = null;

  async function refresh() {
    try {
      jobs = await api.hfDownloadList();
      err = null;
    } catch (e) {
      err = String(e);
    }
  }

  onMount(() => {
    void refresh();
    interval = setInterval(() => {
      void refresh();
    }, 450);
  });

  onDestroy(() => {
    if (interval) clearInterval(interval);
  });

  function formatBytes(n: number | null | undefined): string {
    if (n == null || !Number.isFinite(n) || n < 0) return "—";
    if (n < 1024) return `${Math.round(n)} B`;
    const kb = n / 1024;
    if (kb < 1024) return `${kb < 10 ? kb.toFixed(1) : Math.round(kb)} KB`;
    const mb = kb / 1024;
    if (mb < 1024) return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`;
    return `${(mb / 1024).toFixed(2)} GB`;
  }

  function statusBadgeVariant(
    s: api.HfDownloadJobDto["status"],
  ): "accent" | "muted" {
    if (s === "completed") return "accent";
    return "muted";
  }

  async function pause(id: string) {
    err = null;
    try {
      await api.hfDownloadPause(id);
      await refresh();
    } catch (e) {
      err = String(e);
    }
  }

  async function resume(id: string) {
    err = null;
    try {
      await api.hfDownloadResume(id);
      await refresh();
    } catch (e) {
      err = String(e);
    }
  }

  async function cancel(id: string) {
    err = null;
    try {
      await api.hfDownloadCancel(id);
      await refresh();
    } catch (e) {
      err = String(e);
    }
  }

  async function dismiss(id: string) {
    err = null;
    try {
      await api.hfDownloadDismiss(id);
      await refresh();
    } catch (e) {
      err = String(e);
    }
  }

  const activeCount = $derived(
    jobs.filter((j) =>
      ["queued", "running", "paused"].includes(j.status),
    ).length,
  );
</script>

<div class="flex-1 overflow-y-auto overscroll-y-contain">
  <div class="mx-auto max-w-5xl px-3 py-5 sm:px-6 md:py-8 lg:px-10">
    <PageHeader title="Downloads">
      {#snippet actions()}
        <a
          href="/models"
          class="rounded-xl border-2 border-lm-border px-5 py-3 text-base font-semibold hover:bg-lm-surface-hover"
        >
          Models
        </a>
      {/snippet}
    </PageHeader>

    {#if err}
      <p class="mb-4 rounded-md border border-red-900/50 bg-red-950/40 px-3 py-2 text-sm text-red-200">
        {err}
      </p>
    {/if}

    <p class="mb-6 text-sm text-lm-muted">
      {#if activeCount > 0}
        {activeCount} active · updates every moment
      {:else}
        No active transfers. Start a download from <a class="font-semibold text-lm-accent hover:underline" href="/models">Models</a>.
      {/if}
    </p>

    <div class="space-y-4">
      {#each jobs as j (j.id)}
        <Card>
          <div class="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-center gap-2">
                <h3 class="font-semibold text-lm-text">{j.title}</h3>
                <Badge variant={statusBadgeVariant(j.status)}>{j.status}</Badge>
              </div>
              {#if j.currentFile}
                <p class="mt-1 truncate font-mono text-xs text-lm-muted">
                  {j.currentFile}
                </p>
              {/if}
              <p class="mt-2 text-sm text-lm-muted">{j.message}</p>
              {#if j.error}
                <p class="mt-1 text-sm text-red-300">{j.error}</p>
              {/if}
              {#if j.fileCount > 1}
                <p class="mt-1 text-xs text-lm-muted">
                  File {j.fileIndex} / {j.fileCount}
                </p>
              {/if}
            </div>
            <div class="flex shrink-0 flex-wrap gap-2">
              {#if j.status === "running"}
                <button
                  type="button"
                  class="rounded-md border border-lm-border px-3 py-1.5 text-xs font-semibold hover:bg-lm-surface-hover"
                  onclick={() => pause(j.id)}
                >
                  Pause
                </button>
                <button
                  type="button"
                  class="rounded-md border border-amber-900/50 bg-amber-950/30 px-3 py-1.5 text-xs font-semibold text-amber-100 hover:bg-amber-950/50"
                  onclick={() => cancel(j.id)}
                >
                  Cancel
                </button>
              {:else if j.status === "paused"}
                <button
                  type="button"
                  class="rounded-md border border-lm-border px-3 py-1.5 text-xs font-semibold hover:bg-lm-surface-hover"
                  onclick={() => resume(j.id)}
                >
                  Resume
                </button>
                <button
                  type="button"
                  class="rounded-md border border-amber-900/50 bg-amber-950/30 px-3 py-1.5 text-xs font-semibold text-amber-100 hover:bg-amber-950/50"
                  onclick={() => cancel(j.id)}
                >
                  Cancel
                </button>
              {:else if j.status === "queued"}
                <button
                  type="button"
                  class="rounded-md border border-amber-900/50 bg-amber-950/30 px-3 py-1.5 text-xs font-semibold text-amber-100 hover:bg-amber-950/50"
                  onclick={() => cancel(j.id)}
                >
                  Cancel
                </button>
              {:else if j.status === "completed" || j.status === "cancelled" || j.status === "failed"}
                <button
                  type="button"
                  class="rounded-md border border-lm-border px-3 py-1.5 text-xs font-semibold text-lm-muted hover:bg-lm-surface-hover"
                  onclick={() => dismiss(j.id)}
                >
                  Dismiss
                </button>
              {/if}
            </div>
          </div>
          {#if j.status === "running" || j.status === "paused"}
            <div class="mt-4 space-y-1">
              <div class="flex justify-between text-xs text-lm-muted">
                <span>{Math.round(j.progress)}%</span>
                <span class="tabular-nums">
                  {formatBytes(j.bytesDownloaded)}
                  {#if j.bytesTotal != null}
                    / {formatBytes(j.bytesTotal)}
                  {/if}
                </span>
              </div>
              <ProgressBar value={Math.min(1, j.progress / 100)} />
            </div>
          {/if}
          {#if j.status === "completed" && j.registeredModelId}
            <p class="mt-3 text-xs text-lm-muted">
              Registered in library. Load from <a class="font-semibold text-lm-accent hover:underline" href="/models">Models</a>.
            </p>
          {/if}
        </Card>
      {:else}
        <p class="py-10 text-center text-sm text-lm-muted">
          No download jobs yet.
        </p>
      {/each}
    </div>
  </div>
</div>
