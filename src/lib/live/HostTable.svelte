<script lang="ts">
  import type { HostRow } from "$lib/api";
  import { bytes, count } from "$lib/format";
  import Tile from "./Tile.svelte";

  let { hosts, app, packets }: { hosts: HostRow[]; app: string; packets: boolean } = $props();

  /** A 64 × 16 sparkline of the last 60 seconds. */
  function spark(values: number[]): string {
    const max = Math.max(1, ...values);
    return values
      .map((v, i) => `${((i / (values.length - 1)) * 64).toFixed(1)},${(15 - (v / max) * 13).toFixed(1)}`)
      .join(" ");
  }
</script>

<div class="hosttable">
  <div class="listhead">
    <span>{app ? `${app} is talking to` : "hosts"}</span><span class="spacer"></span><span>sorted by ↓</span>
  </div>
  <div class="hrow head">
    <span></span><span>service · host</span><span>location</span><span>network</span>
    <span class="r">packets</span><span class="r">↓</span><span class="r">↑</span><span>60 s</span>
  </div>
  <div class="rows">
    {#each hosts as h (h.host)}
      {@const network = h.asn ? `AS${h.asn} ${h.network ?? ""}` : ""}
      <div class="hrow">
        <Tile label={h.service ?? "?"} color={h.color} />
        <div class="svc ell" title={h.host}>
          <b class="ell">{h.service ?? h.host}</b>
          {#if h.service && h.service !== h.host}<span class="ell">{h.host}</span>{/if}
        </div>
        <div class="loc">
          {#if h.country}<span class="stamp">{h.country}</span>{/if}
          <span class="ell">{h.city ?? ""}</span>
        </div>
        <div class="ell mute" title={network}>{network}</div>
        <div class="num r">{packets ? count(h.packets) : "n/a"}</div>
        <div class="num r in">{bytes(h.down)}</div>
        <div class="num r out">{bytes(h.up)}</div>
        <svg width="64" height="16" aria-hidden="true">
          <polyline points={spark(h.spark)} fill="none" stroke="var(--in)" stroke-width="1.2" />
        </svg>
      </div>
    {:else}
      <p class="empty mute">Nothing in the last 5 minutes.</p>
    {/each}
  </div>
</div>

<style>
  .hosttable {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }
  .spacer {
    flex: 1;
  }
  .rows {
    overflow-y: auto;
    min-height: 0;
  }
  .hrow {
    display: grid;
    grid-template-columns: 22px minmax(0, 1.7fr) 150px minmax(0, 1fr) 80px 76px 70px 70px;
    gap: 10px;
    align-items: center;
    padding: 6px 14px;
    border-bottom: 1px solid var(--rowline);
  }
  .hrow.head {
    font-size: 11px;
    color: var(--mute);
    text-transform: lowercase;
    padding: 8px 14px;
  }
  .svc {
    display: flex;
    flex-direction: column;
  }
  .svc b {
    font-weight: 600;
  }
  .svc span {
    color: var(--mute);
    font: 11px var(--mono);
  }
  .loc {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .r {
    text-align: right;
  }
  .empty {
    padding: 12px 14px;
    margin: 0;
  }
</style>
