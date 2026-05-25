<script lang="ts">
  let {
    value,
    class: className = "",
  }: {
    value: number;
    class?: string;
  } = $props();

  const pct = $derived(Math.min(100, Math.max(0, value * 100)));
</script>

<div
  class="h-3 w-full overflow-hidden rounded-full border border-lm-border bg-lm-bg {className}"
  role="progressbar"
  aria-valuemin="0"
  aria-valuemax="100"
  aria-valuenow={Math.round(pct)}
>
  <div
    class="h-full rounded-full bg-lm-accent transition-[width] duration-300 ease-out"
    style="width: {pct}%"
  ></div>
</div>
