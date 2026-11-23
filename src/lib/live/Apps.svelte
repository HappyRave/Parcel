<script lang="ts">
  import type { AppRate } from "$lib/api";
  import { rate } from "$lib/format";

  let { apps }: { apps: AppRate[] } = $props();

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
            <td class="r num in">{rate(a.down)}</td>
            <td class="r num out">{rate(a.up)}</td>
            <td class="r num dim">{a.packetsDown + a.packetsUp}</td>
          </tr>
        {:else}
          <tr><td colspan="4" class="dim">No traffic this second.</td></tr>
        {/each}
      </tbody>
    </table>
  </div>

  <div class="panel">
    <h2>{selected ? `${selected.name}: hosts` : "Hosts"}</h2>
    <table>
      <thead><tr><th>Host</th><th class="r">↓</th><th class="r">↑</th></tr></thead>
      <tbody>
        {#each selected?.hosts ?? [] as h (h.host)}
          <tr>
            <td class="name">{h.host}</td>
            <td class="r num in">{rate(h.down)}</td>
            <td class="r num out">{rate(h.up)}</td>
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
