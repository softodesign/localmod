<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    description,
    actions,
  }: {
    title: string;
    description?: string;
    actions?: Snippet;
  } = $props();
</script>

<header class="mb-11 flex flex-wrap items-end justify-between gap-8 md:mb-12">
  <div>
    <h1
      class="text-[2rem] font-bold leading-tight tracking-tight text-lm-text md:text-[2.5rem] md:leading-[1.1]"
    >
      {title}
    </h1>
    {#if description}
      <p
        class="mt-4 max-w-3xl text-lg leading-relaxed text-lm-muted md:text-xl md:leading-relaxed"
      >
        {description}
      </p>
    {/if}
  </div>
  {#if actions}
    <div class="flex flex-wrap items-center gap-4">
      {@render actions()}
    </div>
  {/if}
</header>
