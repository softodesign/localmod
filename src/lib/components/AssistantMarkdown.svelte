<script lang="ts">
  import MarkdownContent from "$lib/components/MarkdownContent.svelte";
  import ChatImageBlock from "$lib/components/ChatImageBlock.svelte";
  import { stripAssistantThinkNoise } from "$lib/chat-assistant";
  import {
    buildAssistantMarkdownBlocks,
    type AssistantMarkdownBlock,
  } from "$lib/chat-markdown";

  let { raw }: { raw: string } = $props();

  let blocks = $state<AssistantMarkdownBlock[]>([]);

  $effect(() => {
    const text = stripAssistantThinkNoise(raw);
    let cancelled = false;
    void buildAssistantMarkdownBlocks(text).then((next) => {
      if (!cancelled) blocks = next;
    });
    return () => {
      cancelled = true;
    };
  });

  const fallbackText = $derived(stripAssistantThinkNoise(raw));
</script>

{#if blocks.length === 0}
  <MarkdownContent source={fallbackText} />
{:else}
  {#each blocks as block, i (i)}
    {#if block.kind === "text"}
      <MarkdownContent source={block.content} />
    {:else}
      <ChatImageBlock
        src={block.src}
        filename={block.filename}
        alt={block.alt}
      />
    {/if}
  {/each}
{/if}
