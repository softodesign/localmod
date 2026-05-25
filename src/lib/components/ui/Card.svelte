<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    children,
    class: className = "",
  }: {
    title?: string;
    children: Snippet;
    class?: string;
  } = $props();
</script>

<section
  class="rounded-card border border-lm-border bg-lm-surface px-6 py-6 md:px-8 md:py-7 {className}"
>
  {#if title}
    <h2 class="mb-5 text-lg font-bold tracking-tight text-lm-text/95">
      {title}
    </h2>
  {/if}
  {@render children()}
</section>
