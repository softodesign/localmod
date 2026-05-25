<script lang="ts">
  type Props = {
    percent: number;
    size?: number;
    /** Shown in hover popover (supports newlines). */
    details?: string;
  };

  let { percent, size = 40, details = "" }: Props = $props();

  const clamped = $derived(Math.min(100, Math.max(0, percent)));
  const strokeW = $derived(size <= 32 ? 2.5 : 3);
  const r = $derived((size - strokeW * 2) / 2);
  const cx = $derived(size / 2);
  const cy = $derived(size / 2);
  const circumference = $derived(2 * Math.PI * r);
  const offset = $derived(circumference * (1 - clamped / 100));
  const stroke = $derived(
    clamped >= 92 ? "#f87171" : clamped >= 75 ? "#fbbf24" : "var(--color-lm-accent, #6ee7b7)",
  );
  const ariaLabel = $derived(
    details.trim() || `Context ${Math.round(clamped)}% used`,
  );
</script>

<div class="group/ctx relative inline-flex shrink-0 items-center justify-center">
  <div
    class="inline-flex items-center justify-center"
    style="width: {size}px; height: {size}px;"
    aria-label={ariaLabel}
  >
    <svg width={size} height={size} class="-rotate-90" aria-hidden="true">
      <circle
        {cx}
        {cy}
        {r}
        fill="none"
        stroke="currentColor"
        stroke-width={strokeW}
        class="text-lm-border/80"
      />
      <circle
        {cx}
        {cy}
        {r}
        fill="none"
        stroke={stroke}
        stroke-width={strokeW}
        stroke-linecap="round"
        stroke-dasharray={circumference}
        stroke-dashoffset={offset}
        class="transition-[stroke-dashoffset] duration-300"
      />
    </svg>
  </div>

  {#if details.trim()}
    <div
      class="pointer-events-none absolute bottom-full right-0 z-30 mb-2 hidden w-[min(16rem,calc(100vw-2rem))] rounded-xl border border-lm-border bg-lm-elevated px-3.5 py-2.5 text-left shadow-lg group-hover/ctx:block"
      role="tooltip"
    >
      <p class="text-xs font-semibold text-lm-text">Context window</p>
      <p class="mt-1.5 whitespace-pre-line text-xs leading-relaxed text-lm-muted">{details}</p>
    </div>
  {/if}
</div>
