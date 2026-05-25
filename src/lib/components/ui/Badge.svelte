<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    children,
    variant = "default",
    class: className = "",
  }: {
    children: Snippet;
    variant?: "default" | "accent" | "warn" | "muted";
    class?: string;
  } = $props();

  const styles = {
    default: "bg-lm-elevated text-lm-text border-lm-border",
    accent: "bg-lm-accent-muted/50 text-lm-accent border-lm-accent/35",
    warn: "bg-amber-950/40 text-amber-100 border-amber-800/45",
    muted: "bg-lm-bg text-lm-muted border-lm-border",
  };
</script>

<span
  class="inline-flex items-center rounded-lg border px-3.5 py-1.5 text-base font-semibold tracking-tight {styles[
    variant
  ]} {className}"
>
  {@render children()}
</span>
