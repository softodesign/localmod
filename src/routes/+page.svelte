<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import ProgressBar from "$lib/components/ui/ProgressBar.svelte";
  import * as api from "$lib/tauri-bridge";

  let dash = $state<api.DashboardDto | null>(null);
  let snap = $state<api.SystemSnapshot | null>(null);
  let err = $state<string | null>(null);
  let snapshotAt = $state<Date | null>(null);

  let interval: ReturnType<typeof setInterval> | null = null;

  function formatTime(d: Date | null) {
    if (!d) return "—";
    return d.toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  }

  async function refreshDashboard() {
    try {
      dash = await api.getDashboard();
    } catch (e) {
      dash = null;
      err = String(e);
    }
  }

  async function refreshSnapshot() {
    try {
      snap = await api.getSystemSnapshot();
      snapshotAt = new Date();
      err = null;
    } catch (e) {
      snap = null;
      err = String(e);
    }
  }

  onMount(async () => {
    await refreshDashboard();
    await refreshSnapshot();
    interval = setInterval(() => {
      void refreshSnapshot();
    }, 2500);
  });

  onDestroy(() => {
    if (interval) clearInterval(interval);
  });

  const ramPct = $derived(
    snap && snap.ramTotalGb > 0 ? Math.min(1, snap.ramUsedGb / snap.ramTotalGb) : 0,
  );
  const cpuPct = $derived(
    snap ? Math.min(1, Math.max(0, snap.cpuUsagePct / 100)) : 0,
  );
  const topDisks = $derived(snap?.disks?.slice(0, 6) ?? []);
  const gpus = $derived(snap?.gpus ?? []);

  function gpuUsagePct(g: api.GpuMetrics): number | null {
    if (g.usagePct == null) return null;
    return Math.min(100, Math.max(0, g.usagePct));
  }

  function gpuVramPct(g: api.GpuMetrics): number | null {
    if (g.vramUsedMb == null || g.vramTotalMb == null || g.vramTotalMb <= 0) return null;
    return Math.min(100, Math.max(0, (g.vramUsedMb / g.vramTotalMb) * 100));
  }

  function formatVramMb(mb: number | null): string {
    if (mb == null) return "—";
    if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
    return `${Math.round(mb)} MB`;
  }
</script>

<div class="flex-1 overflow-y-auto overscroll-y-contain">
  <div class="mx-auto max-w-7xl px-5 py-8 sm:px-7 md:px-12 md:py-12 lg:px-14">
    <PageHeader
      title="Dashboard"
      description="LocalMOD workspace and live system snapshot (CPU, memory, and graphics)."
    />

    {#if err}
      <p
        class="mb-7 rounded-card border border-red-800/50 bg-red-950/35 px-5 py-4 text-base text-red-200"
      >
        {err}
      </p>
    {/if}

    <!-- Quick stats -->
    <div class="mb-9 grid gap-5 sm:grid-cols-2 xl:grid-cols-3">
      <Card class="!p-6 lg:!p-7">
        <p class="text-sm font-semibold uppercase tracking-wide text-lm-muted">Saved chats</p>
        <p class="mt-3 text-4xl font-bold tabular-nums md:text-5xl">
          {dash?.chatCount ?? "—"}
        </p>
        <p class="mt-2 text-base text-lm-muted">Conversations on this device</p>
      </Card>
      <Card class="!p-6 lg:!p-7">
        <p class="text-sm font-semibold uppercase tracking-wide text-lm-muted">GGUF files</p>
        <p class="mt-3 text-4xl font-bold tabular-nums md:text-5xl">
          {dash?.modelCount ?? "—"}
        </p>
        <p class="mt-2 text-base text-lm-muted">Registered in Models</p>
      </Card>
      <Card class="!p-6 lg:!p-7">
        <p class="text-sm font-semibold uppercase tracking-wide text-lm-muted">Snapshot updated</p>
        <p class="mt-3 text-3xl font-bold tabular-nums md:text-4xl">{formatTime(snapshotAt)}</p>
        <p class="mt-2 text-base text-lm-muted">
          Updates about every 2.5s · CPU, RAM & GPU below
        </p>
      </Card>
    </div>

    <div class="grid gap-7 xl:grid-cols-3">
      <div class="space-y-7 xl:col-span-2">
        <Card title="Processor, memory & graphics" class="!p-6 md:!p-7 lg:!p-8">
          {#if snap}
            <div class="flex flex-col gap-6 sm:flex-row sm:items-start sm:justify-between">
              <div class="min-w-0">
                <p class="text-lg font-semibold leading-snug">{snap.cpuName}</p>
                <p class="mt-1.5 text-base text-lm-muted">{snap.cpuCores} logical processors</p>
                {#if snap.hostName || snap.osVersion}
                  <p class="mt-3 text-base text-lm-muted">
                    {#if snap.hostName}<span class="font-medium">{snap.hostName}</span>{/if}
                    {#if snap.hostName && snap.osVersion}<span class="mx-1 opacity-60">·</span>{/if}
                    {#if snap.osVersion}{snap.osVersion}{/if}
                  </p>
                {/if}
              </div>
              <div class="shrink-0 text-right sm:text-left">
                <p class="text-base font-medium text-lm-muted">RAM</p>
                <p class="text-xl font-bold tabular-nums">
                  {snap.ramUsedGb.toFixed(1)} / {snap.ramTotalGb.toFixed(1)} GB
                </p>
              </div>
            </div>
            <div class="mt-7 grid gap-6 sm:grid-cols-2">
              <div class="space-y-2.5">
                <div class="flex justify-between text-base font-medium text-lm-muted">
                  <span>Memory in use</span>
                  <span>{Math.round(ramPct * 100)}%</span>
                </div>
                <ProgressBar value={ramPct} />
              </div>
              <div class="space-y-2.5">
                <div class="flex justify-between text-base font-medium text-lm-muted">
                  <span>CPU load</span>
                  <span>{Math.round(snap.cpuUsagePct)}%</span>
                </div>
                <ProgressBar value={cpuPct} />
              </div>
            </div>

            <div class="mt-8 border-t border-lm-border/80 pt-7">
              <p class="text-base font-semibold text-lm-text">Graphics</p>
              {#if gpus.length === 0}
                <p class="mt-3 text-base text-lm-muted">No GPU detected.</p>
              {:else}
                <ul class="mt-4 space-y-4">
                  {#each gpus as gpu, i (gpu.name + String(i))}
                    {@const usage = gpuUsagePct(gpu)}
                    {@const vramPct = gpuVramPct(gpu)}
                    <li
                      class="rounded-xl border border-lm-border/60 bg-lm-bg/40 px-5 py-4"
                    >
                      <p class="font-semibold leading-snug text-lm-text">{gpu.name}</p>
                      <div class="mt-4 grid gap-4 sm:grid-cols-2">
                        <div class="space-y-2.5">
                          <div class="flex justify-between text-base font-medium text-lm-muted">
                            <span>GPU load</span>
                            <span>{usage != null ? `${Math.round(usage)}%` : "—"}</span>
                          </div>
                          {#if usage != null}
                            <ProgressBar value={usage / 100} />
                          {:else}
                            <p class="text-sm text-lm-muted">Not available for this adapter</p>
                          {/if}
                        </div>
                        <div class="space-y-2.5">
                          <div class="flex justify-between text-base font-medium text-lm-muted">
                            <span>VRAM</span>
                            <span class="tabular-nums">
                              {#if gpu.vramUsedMb != null && gpu.vramTotalMb != null}
                                {formatVramMb(gpu.vramUsedMb)} / {formatVramMb(gpu.vramTotalMb)}
                              {:else}
                                —
                              {/if}
                            </span>
                          </div>
                          {#if vramPct != null}
                            <ProgressBar value={vramPct / 100} />
                          {:else}
                            <p class="text-sm text-lm-muted">Not available for this adapter</p>
                          {/if}
                        </div>
                      </div>
                    </li>
                  {/each}
                </ul>
              {/if}
            </div>

            {#if snap.swapTotalGb > 0.05}
              <p class="mt-6 text-base text-lm-muted">
                Swap · {snap.swapUsedGb.toFixed(1)} / {snap.swapTotalGb.toFixed(1)} GB
              </p>
            {/if}
          {:else}
            <p class="text-base text-lm-muted">Loading…</p>
          {/if}
        </Card>
      </div>

      <div class="space-y-7">
        <Card title="Volumes" class="!p-6 md:!p-7 lg:!p-8">
          {#if topDisks.length === 0}
            <p class="text-base text-lm-muted">No disk info yet.</p>
          {:else}
            <ul class="space-y-4 text-base">
              {#each topDisks as d (d.mount)}
                <li
                  class="rounded-xl border border-lm-border/60 bg-lm-bg/40 px-5 py-4"
                >
                  <div class="flex justify-between gap-2">
                    <span class="truncate font-medium text-lm-muted" title={d.mount}>
                      {d.name || d.mount}
                    </span>
                    <span class="shrink-0 tabular-nums text-base font-bold">
                      {d.freeGb.toFixed(0)} GB free
                    </span>
                  </div>
                  <div class="mt-1.5 text-base text-lm-muted">
                    {d.totalGb.toFixed(0)} GB total · <span class="font-mono text-sm">{d.mount}</span>
                  </div>
                </li>
              {/each}
            </ul>
          {/if}
        </Card>
      </div>
    </div>
  </div>
</div>
