<script lang="ts">
  import { marked } from "marked";

  marked.setOptions({ gfm: true, breaks: true });

  let { source }: { source: string } = $props();

  const html = $derived(
    marked.parse(source ?? "", { async: false }) as string,
  );
</script>

<div
  class="markdown-content text-lg leading-relaxed text-lm-text [&_a]:text-lm-accent [&_a]:underline-offset-2 [&_a]:transition-colors hover:[&_a]:text-lm-accent-hover [&_code]:rounded-lg [&_code]:border [&_code]:border-lm-border [&_code]:bg-lm-bg [&_code]:px-2 [&_code]:py-1 [&_code]:text-[0.95em] [&_h1]:mb-3 [&_h1]:text-3xl [&_h1]:font-bold [&_img]:my-3 [&_img]:max-h-52 [&_img]:max-w-[min(100%,18rem)] [&_img]:rounded-xl [&_img]:border [&_img]:border-lm-border [&_img]:object-contain [&_li]:ml-4 [&_li]:list-disc [&_ol]:ml-4 [&_ol]:list-decimal [&_p]:mb-3 [&_pre]:my-3 [&_pre]:overflow-x-auto [&_pre]:rounded-xl [&_pre]:border [&_pre]:border-lm-border [&_pre]:bg-lm-bg [&_pre]:p-4 [&_pre]:text-[0.95rem] [&_strong]:font-bold"
>
  {@html html}
</div>
