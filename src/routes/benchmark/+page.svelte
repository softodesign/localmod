<script lang="ts">
  import { onMount } from "svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import * as api from "$lib/tauri-bridge";
  import { isLoadableModel } from "$lib/model-utils";

  let models = $state<api.ModelDto[]>([]);
  let primaryModelId = $state("");
  let secondaryModelId = $state("");
  let running = $state(false);
  let err = $state<string | null>(null);
  let result = $state<api.BenchmarkRunDto | null>(null);

  const benchmarkableModels = $derived(
    models.filter((m) => isLoadableModel(m) && m.weightsFormat !== "safetensors"),
  );

  const selectedIds = $derived.by(() => {
    const ids = [primaryModelId, secondaryModelId]
      .map((id) => id.trim())
      .filter(Boolean);
    return [...new Set(ids)].slice(0, 2);
  });

  const canRun = $derived(selectedIds.length > 0 && !running);

  function fmtMs(ms: number): string {
    if (!Number.isFinite(ms)) return "-";
    if (ms >= 1000) return `${(ms / 1000).toFixed(2)}s`;
    return `${Math.round(ms)}ms`;
  }

  function fmtNum(n: number, digits = 1): string {
    if (!Number.isFinite(n)) return "-";
    return n.toFixed(digits);
  }

  function formatModelLabel(m: api.ModelDto): string {
    const type = m.weightsFormat === "cloud" ? "cloud" : "local";
    return `${m.name} (${type})`;
  }

  async function refreshModels() {
    models = await api.listModels();
    if (!primaryModelId) primaryModelId = benchmarkableModels[0]?.id ?? "";
    if (!secondaryModelId) {
      const next = benchmarkableModels.find((m) => m.id !== primaryModelId);
      secondaryModelId = next?.id ?? "";
    }
  }

  async function runBenchmark() {
    if (!canRun) return;
    running = true;
    err = null;
    result = null;
    try {
      result = await api.runModelBenchmark(selectedIds);
    } catch (e) {
      err = String(e);
    } finally {
      running = false;
    }
  }

  onMount(() => {
    void refreshModels();
  });
</script>

<div class="flex-1 overflow-y-auto overscroll-y-contain">
  <div class="mx-auto max-w-7xl px-4 py-7 sm:px-8 md:py-10 lg:px-12">
    <PageHeader title="Benchmark"></PageHeader>

    {#if err}
      <p class="mb-4 rounded-md border border-red-900/50 bg-red-950/40 px-3 py-2 text-sm text-red-200">
        {err}
      </p>
    {/if}

    <div class="grid gap-4 lg:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
      <Card title="Run model benchmark">
        <p class="mb-4 text-sm leading-relaxed text-lm-muted">
          Select one or two models. Local GGUFs are loaded into llama-server before testing;
          cloud models use their configured provider API. Results estimate speed, latency,
          memory delta, reasoning, and coding behavior.
        </p>

        <div class="space-y-4">
          <div>
            <label class="block text-sm font-semibold text-lm-muted" for="bench-primary">
              Model A
            </label>
            <select
              id="bench-primary"
              bind:value={primaryModelId}
              disabled={running}
              class="mt-1 w-full rounded-xl border border-lm-border bg-lm-bg px-4 py-3 text-base"
            >
              <option value="">Choose a model...</option>
              {#each benchmarkableModels as m (m.id)}
                <option value={m.id}>{formatModelLabel(m)}</option>
              {/each}
            </select>
          </div>

          <div>
            <label class="block text-sm font-semibold text-lm-muted" for="bench-secondary">
              Model B <span class="font-normal">(optional)</span>
            </label>
            <select
              id="bench-secondary"
              bind:value={secondaryModelId}
              disabled={running}
              class="mt-1 w-full rounded-xl border border-lm-border bg-lm-bg px-4 py-3 text-base"
            >
              <option value="">None</option>
              {#each benchmarkableModels as m (m.id)}
                <option value={m.id}>{formatModelLabel(m)}</option>
              {/each}
            </select>
          </div>

          <button
            type="button"
            class="min-h-12 rounded-xl border border-lm-accent bg-lm-accent px-5 py-2.5 text-base font-bold text-lm-bg hover:bg-lm-accent-hover disabled:opacity-50"
            disabled={!canRun}
            onclick={() => void runBenchmark()}
          >
            {running ? "Running benchmark..." : selectedIds.length > 1 ? "Benchmark both models" : "Benchmark model"}
          </button>
        </div>
      </Card>

      <Card title="Benchmark suite">
        <div class="grid gap-3 sm:grid-cols-3">
          <div class="rounded-xl border border-lm-border bg-lm-bg px-4 py-3">
            <Badge variant="muted">latency</Badge>
            <p class="mt-2 text-sm text-lm-muted">Short response round trip.</p>
          </div>
          <div class="rounded-xl border border-lm-border bg-lm-bg px-4 py-3">
            <Badge variant="muted">reasoning</Badge>
            <p class="mt-2 text-sm text-lm-muted">Math word problem with steps.</p>
          </div>
          <div class="rounded-xl border border-lm-border bg-lm-bg px-4 py-3">
            <Badge variant="muted">coding</Badge>
            <p class="mt-2 text-sm text-lm-muted">TypeScript helper implementation.</p>
          </div>
        </div>
        <p class="mt-4 text-xs leading-relaxed text-lm-muted">
          Token counts are estimated from output length, so use them as a practical comparison
          signal rather than tokenizer-perfect accounting.
        </p>
      </Card>
    </div>

    {#if running}
      <div class="mt-6 rounded-2xl border border-lm-border bg-lm-surface px-5 py-4 text-lm-muted">
        Running prompts. Local models may take a while to load before the first result.
      </div>
    {/if}

    {#if result}
      <div class="mt-6 space-y-5">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <h2 class="text-lg font-bold text-lm-text">Results</h2>
          <span class="text-xs text-lm-muted">Run {new Date(result.createdAt).toLocaleString()}</span>
        </div>

        <div class="grid gap-4 lg:grid-cols-2">
          {#each result.models as model (model.modelId)}
            <Card title={model.modelName}>
              <div class="grid grid-cols-2 gap-3 sm:grid-cols-3">
                <div class="rounded-xl border border-lm-border bg-lm-bg px-3 py-2">
                  <p class="text-xs text-lm-muted">Avg latency</p>
                  <p class="mt-1 font-bold">{fmtMs(model.avgLatencyMs)}</p>
                </div>
                <div class="rounded-xl border border-lm-border bg-lm-bg px-3 py-2">
                  <p class="text-xs text-lm-muted">Speed</p>
                  <p class="mt-1 font-bold">{fmtNum(model.avgTokensPerSecond)} tok/s</p>
                </div>
                <div class="rounded-xl border border-lm-border bg-lm-bg px-3 py-2">
                  <p class="text-xs text-lm-muted">Memory delta</p>
                  <p class="mt-1 font-bold">{fmtNum(model.ramDeltaGb, 2)} GB</p>
                </div>
                <div class="rounded-xl border border-lm-border bg-lm-bg px-3 py-2">
                  <p class="text-xs text-lm-muted">Load time</p>
                  <p class="mt-1 font-bold">{fmtMs(model.loadMs)}</p>
                </div>
                <div class="rounded-xl border border-lm-border bg-lm-bg px-3 py-2">
                  <p class="text-xs text-lm-muted">Total time</p>
                  <p class="mt-1 font-bold">{fmtMs(model.totalMs)}</p>
                </div>
                <div class="rounded-xl border border-lm-border bg-lm-bg px-3 py-2">
                  <p class="text-xs text-lm-muted">Output</p>
                  <p class="mt-1 font-bold">~{model.totalEstimatedTokens} tok</p>
                </div>
              </div>

              <div class="mt-4 space-y-3">
                {#each model.prompts as p (p.id)}
                  <details class="rounded-xl border border-lm-border bg-lm-bg px-4 py-3">
                    <summary class="cursor-pointer text-sm font-semibold text-lm-text">
                      {p.title} - {fmtMs(p.latencyMs)} - {fmtNum(p.tokensPerSecond)} tok/s
                    </summary>
                    {#if p.error}
                      <p class="mt-3 rounded-lg border border-red-900/40 bg-red-950/30 px-3 py-2 text-sm text-red-200">
                        {p.error}
                      </p>
                    {:else}
                      <p class="mt-3 whitespace-pre-wrap text-sm leading-relaxed text-lm-muted">
                        {p.output}
                      </p>
                    {/if}
                  </details>
                {/each}
              </div>
            </Card>
          {/each}
        </div>
      </div>
    {/if}
  </div>
</div>
