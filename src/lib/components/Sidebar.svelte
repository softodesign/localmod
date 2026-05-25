<script lang="ts">
  import { page } from "$app/state";
  import LayoutDashboard from "@lucide/svelte/icons/layout-dashboard";
  import MessageSquare from "@lucide/svelte/icons/message-square";
  import Box from "@lucide/svelte/icons/box";
  import Gauge from "@lucide/svelte/icons/gauge";
  import FileText from "@lucide/svelte/icons/file-text";
  import Download from "@lucide/svelte/icons/download";
  import Settings from "@lucide/svelte/icons/settings";

  const items = [
    { href: "/", label: "Home", Icon: LayoutDashboard },
    { href: "/chats", label: "Chats", Icon: MessageSquare },
    { href: "/models", label: "Models", Icon: Box },
    { href: "/benchmark", label: "Benchmark", Icon: Gauge },
    { href: "/context", label: "Reference", Icon: FileText },
    { href: "/downloads", label: "Downloads", Icon: Download },
    { href: "/settings", label: "Settings", Icon: Settings },
  ] as const;
</script>

<aside
  class="flex h-full w-[20rem] shrink-0 flex-col border-r border-lm-border bg-lm-surface md:w-[21.5rem]"
>
  <div class="flex items-center gap-4 border-b border-lm-border px-5 py-5 md:px-6 md:py-5">
    <img
      src="/LocalMOD.png"
      width="52"
      height="52"
      alt="LocalMOD"
      class="size-[3.25rem] shrink-0 rounded-xl border border-lm-border bg-lm-elevated object-contain"
      draggable="false"
    />
    <div class="min-w-0">
      <div class="truncate text-[1.0625rem] font-bold leading-tight tracking-tight text-lm-text">
        LocalMOD
      </div>
      <div class="mt-1 truncate text-[0.9375rem] leading-snug text-lm-muted">
        Private · runs on your device
      </div>
    </div>
  </div>

  <nav class="flex flex-1 flex-col gap-1 p-4 md:p-4" aria-label="Main">
    {#each items as item (item.href)}
      {@const active =
        item.href === "/"
          ? page.url.pathname === "/"
          : page.url.pathname === item.href ||
            page.url.pathname.startsWith(item.href + "/")}
      {@const Icon = item.Icon}
      <a
        href={item.href}
        class="group relative flex min-h-[3.25rem] items-center gap-3.5 rounded-xl border px-4 py-3 text-[1.0625rem] font-semibold tracking-tight transition-[background-color,border-color,color] duration-150 ease-out active:scale-[0.99] {active
          ? 'border-lm-border bg-lm-bg text-lm-text'
          : 'border-transparent text-lm-muted hover:border-lm-border hover:bg-lm-surface-hover hover:text-lm-text'}"
      >
        <Icon
          class="size-6 shrink-0 transition-colors duration-150 {active
            ? 'text-lm-accent'
            : 'opacity-85 group-hover:opacity-100'}"
          strokeWidth={2}
        />
        <span>{item.label}</span>
        {#if active}
          <span
            class="absolute start-2 top-1/2 h-8 w-0.5 -translate-y-1/2 rounded-full bg-lm-accent"
            aria-hidden="true"
          ></span>
        {/if}
      </a>
    {/each}
  </nav>

  <div
    class="mx-4 mb-4 mt-auto rounded-xl border border-lm-border/80 bg-lm-elevated/60 px-4 py-3 text-center text-sm font-medium tabular-nums text-lm-muted"
  >
    v0.1.0
  </div>
</aside>
