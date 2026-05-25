<script lang="ts">
  import { save } from "@tauri-apps/plugin-dialog";
  import Download from "@lucide/svelte/icons/download";
  import * as api from "$lib/tauri-bridge";

  let {
    src,
    filename,
    alt = "Generated image",
  }: {
    src: string;
    filename: string;
    alt?: string;
  } = $props();

  let busy = $state(false);
  let err = $state<string | null>(null);

  async function download() {
    err = null;
    busy = true;
    try {
      const dest = await save({
        defaultPath: filename,
        filters: [{ name: "PNG image", extensions: ["png"] }],
      });
      if (dest) {
        await api.exportGeneratedImage(filename, dest);
      }
    } catch (e) {
      err = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<figure class="my-3 inline-flex max-w-full flex-col gap-2">
  <img
    {src}
    {alt}
    class="max-h-52 max-w-[min(100%,18rem)] rounded-xl border border-lm-border bg-lm-bg object-contain"
    loading="lazy"
  />
  <div class="flex flex-wrap items-center gap-2">
    <button
      type="button"
      class="inline-flex items-center gap-1.5 rounded-lg border border-lm-border bg-lm-surface px-3 py-1.5 text-sm font-semibold text-lm-muted hover:bg-lm-surface-hover hover:text-lm-text disabled:opacity-50"
      disabled={busy}
      onclick={() => void download()}
    >
      <Download class="size-3.5" strokeWidth={2} />
      {busy ? "Saving…" : "Download"}
    </button>
    {#if err}
      <span class="text-xs text-red-300">{err}</span>
    {/if}
  </div>
</figure>
