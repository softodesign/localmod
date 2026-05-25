<script lang="ts">
  import { goto } from "$app/navigation";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import * as api from "$lib/tauri-bridge";

  let title = $state("");
  let description = $state("");
  let content = $state("");
  let busy = $state(false);
  let err = $state<string | null>(null);

  async function save() {
    err = null;
    if (!content.trim()) {
      err = "Please enter the main text.";
      return;
    }
    busy = true;
    try {
      await api.addContextText(title, description, content);
      await goto("/context");
    } catch (e) {
      err = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="flex-1 overflow-y-auto overscroll-y-contain">
  <div class="mx-auto max-w-xl px-4 py-8 sm:px-6 md:py-10">
    <PageHeader
      title="Add manually"
      description="Give it a title and optional short description, then add the text you want kept in your library."
    />

    {#if err}
      <p
        class="mb-6 rounded-md border border-red-900/50 bg-red-950/35 px-4 py-3 text-sm text-red-200"
      >
        {err}
      </p>
    {/if}

    <Card title="New item">
      <div class="space-y-4">
        <label class="block">
          <span class="text-sm font-medium text-lm-muted">Title</span>
          <input
            type="text"
            bind:value={title}
            disabled={busy}
            placeholder="e.g. Wi‑Fi password"
            class="mt-1.5 w-full rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2.5 text-sm"
          />
        </label>
        <label class="block">
          <span class="text-sm font-medium text-lm-muted">Description (optional)</span>
          <textarea
            bind:value={description}
            disabled={busy}
            rows="3"
            placeholder="A few words about what this is"
            class="mt-1.5 w-full resize-y rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2.5 text-sm"
          ></textarea>
        </label>
        <label class="block">
          <span class="text-sm font-medium text-lm-muted">Main text</span>
          <textarea
            bind:value={content}
            disabled={busy}
            rows="12"
            placeholder="The content you want to save"
            class="mt-1.5 w-full resize-y rounded-md border-2 border-lm-border bg-lm-bg px-3 py-2.5 text-sm"
          ></textarea>
        </label>
      </div>

      <div class="mt-6 flex flex-wrap gap-3">
        <button
          type="button"
          disabled={busy}
          class="rounded-xl border-2 border-lm-accent bg-lm-accent px-7 py-3.5 text-base font-bold text-lm-bg hover:bg-lm-accent-hover disabled:opacity-50"
          onclick={() => save()}
        >
          {busy ? "Saving…" : "Save to library"}
        </button>
        <a
          href="/context"
          class="inline-flex items-center rounded-xl border-2 border-lm-border px-7 py-3.5 text-base font-semibold hover:bg-lm-surface-hover"
        >
          Cancel
        </a>
      </div>
    </Card>
  </div>
</div>
