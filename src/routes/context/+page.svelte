<script lang="ts">
  import { onMount } from "svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";
  import * as api from "$lib/tauri-bridge";

  let docs = $state<api.ContextDocDto[]>([]);
  let err = $state<string | null>(null);

  function statusVariant(s: string) {
    if (s === "ready") return "accent" as const;
    return "warn" as const;
  }

  async function refresh() {
    err = null;
    docs = await api.listContextDocuments();
  }

  async function removeDoc(id: string) {
    if (!confirm("Remove this from your library?")) return;
    err = null;
    try {
      await api.deleteContextDocument(id);
      await refresh();
    } catch (e) {
      err = String(e);
    }
  }

  onMount(() => {
    void refresh();
  });
</script>

<div class="flex-1 overflow-y-auto overscroll-y-contain">
  <div class="mx-auto max-w-3xl px-5 py-9 sm:px-8 md:py-12">
    <PageHeader
      title="Reference"
      description="Add your own notes and snippets so the app can use them in context. Everything stays on your device."
    />

    {#if err}
      <p
        class="mb-7 rounded-lg border border-red-900/50 bg-red-950/35 px-5 py-4 text-base text-red-200"
      >
        {err}
      </p>
    {/if}

    <a
      href="/context/add"
      class="flex min-h-[3.75rem] w-full items-center justify-center rounded-xl border border-lm-accent bg-lm-accent px-6 py-4 text-center text-base font-bold text-lm-bg transition-[background-color,border-color,filter] duration-150 hover:border-lm-accent-hover hover:bg-lm-accent-hover"
    >
      Add reference text
    </a>

    <p class="mt-5 text-center text-sm text-lm-muted">
      Enter a title, short description, and body — it is stored only in your local library.
    </p>

    <Card title="Your library" class="mt-11">
      {#if docs.length === 0}
        <p class="py-14 text-center text-base text-lm-muted">
          Nothing here yet — use “Add reference text” to create your first entry.
        </p>
      {:else}
        <ul class="divide-y divide-lm-border">
          {#each docs as d (d.id)}
            <li class="flex flex-col gap-2 py-4 sm:flex-row sm:items-start sm:justify-between">
              <div class="min-w-0">
                <p class="font-semibold text-lm-text">{d.name}</p>
                <p class="mt-1 text-xs text-lm-muted">
                  {d.kind} · {d.source}
                  {#if d.sizeBytes != null}
                    · ~{Math.round(d.sizeBytes / 1024)} KB
                  {/if}
                </p>
                <div class="mt-2">
                  <Badge variant={statusVariant(d.status)}>{d.status}</Badge>
                </div>
              </div>
              <div class="flex shrink-0 flex-col gap-2 self-start sm:items-end">
                {#if d.source === "text"}
                  <a
                    href="/context/edit/{d.id}"
                    class="text-base font-semibold text-lm-accent hover:underline"
                  >
                    Edit
                  </a>
                {/if}
                <button
                  type="button"
                  class="text-base font-semibold text-lm-accent hover:underline"
                  onclick={() => removeDoc(d.id)}
                >
                  Remove
                </button>
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </Card>
  </div>
</div>
