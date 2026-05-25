<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { onMount } from "svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import * as api from "$lib/tauri-bridge";

  const docId = $derived(page.params.id ?? "");

  let title = $state("");
  let description = $state("");
  let content = $state("");
  let busy = $state(false);
  let err = $state<string | null>(null);
  let loadErr = $state<string | null>(null);

  async function load() {
    loadErr = null;
    if (!docId) {
      loadErr = "Missing id.";
      return;
    }
    try {
      const d = await api.getContextTextForEdit(docId);
      title = d.name;
      description = d.description;
      content = d.content;
    } catch (e) {
      loadErr = String(e);
    }
  }

  onMount(() => {
    void load();
  });

  async function save() {
    err = null;
    if (!content.trim()) {
      err = "Please enter the main text.";
      return;
    }
    busy = true;
    try {
      await api.updateContextText(docId, title, description, content);
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
      title="Edit reference"
      description="Update the title, description, and body. This overwrites the stored file on your device."
    />

    {#if loadErr}
      <p
        class="mb-6 rounded-md border border-red-900/50 bg-red-950/35 px-4 py-3 text-sm text-red-200"
      >
        {loadErr}
      </p>
      <a
        href="/context"
        class="text-sm font-semibold text-lm-accent hover:underline">← Back to Reference</a>
    {:else}
      {#if err}
        <p
          class="mb-6 rounded-md border border-red-900/50 bg-red-950/35 px-4 py-3 text-sm text-red-200"
        >
          {err}
        </p>
      {/if}

      <Card title="Item">
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
          <a
            href="/context"
            class="inline-flex min-h-14 items-center rounded-xl border-2 border-lm-border px-5 py-3 text-base font-semibold hover:bg-lm-surface-hover"
          >
            Cancel
          </a>
          <button
            type="button"
            class="min-h-14 rounded-xl border-2 border-lm-accent bg-lm-accent px-6 py-3 text-base font-bold text-lm-bg hover:bg-lm-accent-hover disabled:opacity-50"
            disabled={busy}
            onclick={() => void save()}
          >
            {busy ? "Saving…" : "Save changes"}
          </button>
        </div>
      </Card>
    {/if}
  </div>
</div>
