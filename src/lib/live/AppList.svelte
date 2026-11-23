<script lang="ts">
  import type { AppRow } from "$lib/api";
  import { bytes } from "$lib/format";
  import Tile from "./Tile.svelte";

  let {
    apps,
    selected,
    onpick,
  }: { apps: AppRow[]; selected: string | null; onpick: (path: string) => void } = $props();

  const max = $derived(Math.max(1, ...apps.map((a) => a.down + a.up)));
</script>

<div class="applist">
  <div class="listhead"><span>apps</span><span class="spacer"></span><span>last 5 min</span></div>
  <div class="rows">
    {#each apps as a (a.path)}
      <button class="approw" class:on={a.path === selected} onclick={() => onpick(a.path)}>
        <Tile label={a.name} color={a.color} large />
        <span class="ell" title={a.path}><b>{a.name}</b> <span class="mute num exe">{a.exe}</span></span>
        <span class="num in val">{bytes(a.down)}</span>
        <span class="bar">
          <b style:width="{(a.down / max) * 100}%" style:background="var(--in)"></b>
          <b style:width="{(a.up / max) * 100}%" style:background="var(--out)"></b>
        </span>
      </button>
    {:else}
      <p class="empty mute">No traffic in the last 5 minutes.</p>
    {/each}
  </div>
</div>

<style>
  .applist {
    border-right: 1px solid var(--line);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .spacer {
    flex: 1;
  }
  .rows {
    overflow-y: auto;
    min-height: 0;
  }
  .approw {
    width: 100%;
    display: grid;
    grid-template-columns: 22px 1fr auto;
    gap: 4px 10px;
    align-items: center;
    padding: 7px 14px;
    border: 0;
    border-bottom: 1px solid var(--rowline);
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: default;
  }
  .approw.on {
    background: var(--raise);
    box-shadow: inset 2px 0 0 var(--kraft);
  }
  b {
    font-weight: 600;
  }
  .exe {
    font-size: 11px;
  }
  .val {
    font-size: 12px;
  }
  .bar {
    grid-column: 2 / 4;
    height: 4px;
    border-radius: 2px;
    background: var(--line);
    overflow: hidden;
    display: flex;
  }
  .bar b {
    display: block;
    height: 100%;
  }
  .empty {
    padding: 12px 14px;
    margin: 0;
  }
</style>
