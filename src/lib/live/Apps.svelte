<script lang="ts">
  import type { AppRate } from "$lib/api";
  import { rate } from "$lib/format";

  let {
    apps,
    amount = rate,
    empty = "No traffic this second.",
  }: { apps: AppRate[]; amount?: (bytes: number) => string; empty?: string } = $props();

  let picked = $state<string | null>(null);
  const selected = $derived(apps.find((a) => a.path === picked) ?? apps[0]);
</script>

<section class="split">
  <div class="panel">
    <h2>Apps</h2>
    <table>
      <thead><tr><th>App</th><th class="r">↓</th><th class="r">↑</th><th class="r">Packets</th></tr></thead>
      <tbody>
        {#each apps as a (a.path)}
          <tr class:on={a === selected} onclick={() => (picked = a.path)}>
            <td class="name" title={a.path}>{a.name}</td>
            <td class="r num in">{amount(a.down)}</td>
            <td class="r num out">{amount(a.up)}</td>
            <td class="r num dim">{a.packetsDown + a.packetsUp}</td>
          </tr>
        {:else}
          <tr><td colspan="4" class="dim">{empty}</td></tr>
        {/each}
      </tbody>
    </table>
  </div>

  <div class="panel">
    <h2>{selected ? `${selected.name}: hosts` : "Hosts"}</h2>
    <table>
      <thead>
        <tr>
          <th>Service · host</th><th>Location</th><th>Network</th>
          <th class="r">↓</th><th class="r">↑</th><th class="r">Packets</th>
        </tr>
      </thead>
      <tbody>
        {#each selected?.hosts ?? [] as h (h.host)}
          {@const network = h.asn ? `AS${h.asn} ${h.network ?? ""}` : ""}
          <tr>
            <td class="name svc" title={h.host}>
              <b>{h.service ?? h.host}</b>
              {#if h.service && h.service !== h.host}<span>{h.host}</span>{/if}
            </td>
            <td class="loc">
              {#if h.country}<i class="stamp">{h.country}</i>{/if}
              <span class="clip">{h.city ?? ""}</span>
            </td>
            <td class="clip dim" title={network}>{network}</td>
            <td class="r num in">{amount(h.down)}</td>
            <td class="r num out">{amount(h.up)}</td>
            <td class="r num dim">{h.packetsDown + h.packetsUp}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</section>

<style>
  .split {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    margin-top: 12px;
    min-height: 0;
  }
  .panel {
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 10px 12px;
    overflow: auto;
  }
  h2 {
    margin: 0 0 6px;
    font: 600 11px var(--mono);
    color: var(--kraft);
    letter-spacing: 0.5px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th {
    font-weight: 400;
    font-size: 11px;
    color: var(--mute);
    text-align: left;
    padding: 2px 6px;
  }
  td {
    padding: 4px 6px;
    border-top: 1px solid var(--line);
  }
  tbody tr {
    cursor: default;
  }
  tbody tr.on {
    background: var(--raise);
  }
  .name {
    max-width: 0;
    width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .loc {
    white-space: nowrap;
  }
  .clip {
    display: inline-block;
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    vertical-align: bottom;
  }
  td.clip {
    display: table-cell;
  }
  /* R2 P2: postage-stamp country codes, straight. */
  .stamp {
    font: normal 700 10px var(--mono);
    letter-spacing: 0.5px;
    padding: 0 4px;
    margin-right: 6px;
    color: var(--kraft);
    border: 1.5px dotted var(--kraft);
    border-radius: 2px;
    line-height: 13px;
  }
  .svc b {
    font-weight: 600;
    margin-right: 6px;
  }
  .svc span {
    font: 11px var(--mono);
    color: var(--mute);
  }
  .r {
    text-align: right;
    white-space: nowrap;
  }
  .in {
    color: var(--in);
  }
  .out {
    color: var(--out);
  }
  .dim {
    color: var(--dim);
  }
</style>
