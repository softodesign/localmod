<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import * as api from "$lib/tauri-bridge";
  import { applyTheme } from "$lib/theme";
  import type { Snippet } from "svelte";

  let { children }: { children: Snippet } = $props();

  function isEditableTarget(t: EventTarget | null): boolean {
    const el = t as HTMLElement | null;
    if (!el) return false;
    if (el.isContentEditable) return true;
    const tag = el.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
    return Boolean(el.closest("input, textarea, select, [contenteditable='true']"));
  }

  /** True inside the Tauri webview (dev or release). Plain Vite in a browser stays unlocked. */
  function isTauriWebview(): boolean {
    return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  }

  onMount(() => {
    const lock = import.meta.env.PROD || isTauriWebview();
    if (lock) {
      document.documentElement.classList.add("lm-shell-lock");
    }

    void (async () => {
      try {
        const rows = await api.getSettings();
        const m = Object.fromEntries(rows);
        applyTheme(m.theme ?? "dark");
      } catch {
        applyTheme("dark");
      }
    })();

    function onKeyDown(e: KeyboardEvent) {
      if (!lock) return;
      const k = e.key.toLowerCase();
      const mod = e.ctrlKey || e.metaKey;

      if (k === "r" && mod) {
        e.preventDefault();
        return;
      }
      if (e.key === "F5") {
        e.preventDefault();
        return;
      }
      if (k === "i" && mod && e.shiftKey) {
        e.preventDefault();
        return;
      }
      if (k === "j" && mod && e.shiftKey) {
        e.preventDefault();
        return;
      }
      if (k === "c" && mod && e.shiftKey) {
        e.preventDefault();
        return;
      }
      if (k === "u" && mod) {
        e.preventDefault();
        return;
      }
      if (e.key === "F12") {
        e.preventDefault();
        return;
      }
      if (k === "a" && mod && !isEditableTarget(e.target)) {
        e.preventDefault();
        return;
      }
      if (k === "s" && mod && !isEditableTarget(e.target)) {
        e.preventDefault();
        return;
      }
      if (k === "p" && mod && !isEditableTarget(e.target)) {
        e.preventDefault();
        return;
      }
    }

    function onContextMenu(e: MouseEvent) {
      if (!lock) return;
      e.preventDefault();
    }

    function onSelectStart(e: Event) {
      if (!lock) return;
      if (!isEditableTarget(e.target)) e.preventDefault();
    }

    function onDragStart(e: DragEvent) {
      if (!lock) return;
      if (!isEditableTarget(e.target)) e.preventDefault();
    }

    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("contextmenu", onContextMenu, true);
    document.addEventListener("selectstart", onSelectStart, true);
    document.addEventListener("dragstart", onDragStart, true);
    return () => {
      if (lock) {
        document.documentElement.classList.remove("lm-shell-lock");
      }
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("contextmenu", onContextMenu, true);
      document.removeEventListener("selectstart", onSelectStart, true);
      document.removeEventListener("dragstart", onDragStart, true);
    };
  });
</script>

<div class="flex h-screen overflow-hidden bg-lm-bg">
  <Sidebar />
  <main class="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
    {@render children()}
  </main>
</div>
