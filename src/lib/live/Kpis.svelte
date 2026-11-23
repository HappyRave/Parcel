<script lang="ts">
  import type { Kpis } from "$lib/api";
  import { count, parts } from "$lib/format";

  let { kpis }: { kpis: Kpis } = $props();

  const cells = $derived.by(() => {
    const [d, du] = parts(kpis.down);
    const [u, uu] = parts(kpis.up);
    const [td, tdu] = parts(kpis.todayDown);
    const [tu, tuu] = parts(kpis.todayUp);
    return [
      { k: "download", v: d, unit: `${du}/s`, cls: "in" },
      { k: "upload", v: u, unit: `${uu}/s`, cls: "out" },
      { k: "today ↓", v: td, unit: tdu, cls: "" },
      { k: "today ↑", v: tu, unit: tuu, cls: "" },
      {
        k: "packets / min",
        v: kpis.packetsPerMin === null ? "n/a" : count(kpis.packetsPerMin),
        unit: "",
        cls: "",
      },
      {
        k: "talking now",
        v: String(kpis.appsNow),
        unit: `apps · ${kpis.hostsNow} hosts`,
        cls: "",
      },
    ];
  });
</script>

<div class="kpis">
  {#each cells as c (c.k)}
    <div class="kpi">
      <div class="k">{c.k}</div>
      <div class="v num {c.cls}">{c.v} <small>{c.unit}</small></div>
    </div>
  {/each}
</div>

<style>
  .kpis {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 1fr;
    border-bottom: 1px solid var(--line);
    flex: none;
  }
  .kpi {
    padding: 10px 16px;
    border-right: 1px solid var(--line);
    min-width: 0;
  }
  .kpi:last-child {
    border-right: 0;
  }
  .k {
    font-size: 11px;
    color: var(--mute);
    text-transform: lowercase;
  }
  .v {
    font-size: 20px;
    font-weight: 600;
    white-space: nowrap;
  }
  .v small {
    font-size: 12px;
    color: var(--mute);
    font-weight: 400;
  }
</style>
