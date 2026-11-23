<script lang="ts">
  import { paint, type Graph } from "$lib/api";
  import { clock, rate } from "$lib/format";

  let { graph }: { graph: Graph } = $props();

  // R2 / R3 H3: download above the line; upload below on its own, shorter, paler scale.
  const H = 220;
  const MID = H * 0.72;
  const FOOT = 18;
  const D_EXT = MID - 22;
  const U_EXT = H - MID - FOOT;
  const TICKS = ["-5:00", "-4:00", "-3:00", "-2:00", "-1:00", "now"];

  let mode = $state<"apps" | "two">("apps");
  let width = $state(0);
  let hover = $state<number | null>(null);

  const n = $derived(graph.down.length);
  const x = (i: number) => (n < 2 ? 0 : (i / (n - 1)) * width);
  const yDown = (v: number) => MID - (v / Math.max(graph.downMax, 1)) * D_EXT;
  const yUp = (v: number) => MID + (v / Math.max(graph.upMax, 1)) * U_EXT;

  /** One stacked band between `base` and `base + vals`, returning the new top. */
  function band(vals: number[], base: number[], y: (v: number) => number) {
    const top = vals.map((v, i) => base[i] + v);
    const fwd = top.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`);
    const back = base.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`).reverse();
    return { points: `${fwd.join(" ")} ${back.join(" ")}`, top };
  }

  const stacks = $derived.by(() => {
    let down = new Array<number>(n).fill(0);
    let up = new Array<number>(n).fill(0);
    return graph.bands.map((b) => {
      const d = band(b.down, down, yDown);
      const u = band(b.up, up, yUp);
      down = d.top;
      up = u.top;
      return { fill: paint(b.color), down: d.points, up: u.points };
    });
  });

  const line = (vals: number[], y: (v: number) => number) =>
    vals.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`).join(" ");
  const twoDown = $derived(line(graph.down, yDown));
  const twoUp = $derived(line(graph.up, yUp));

  function move(e: MouseEvent) {
    const r = (e.currentTarget as SVGElement).getBoundingClientRect();
    const i = Math.round(((e.clientX - r.left) / r.width) * (n - 1));
    hover = Math.max(0, Math.min(n - 1, i));
  }

  const tipX = $derived(hover === null ? 0 : Math.min(x(hover) + 8, width - 240));
</script>

<div class="graphwrap" bind:clientWidth={width}>
  <div class="legend">
    {#if mode === "apps"}
      {#each graph.bands as b (b.name)}
        <span><i style:background={paint(b.color)}></i>{b.name}</span>
      {/each}
    {:else}
      <span class="in">▲ download</span><span class="out">▼ upload</span>
    {/if}
  </div>
  <div class="bar">
    <button class:on={mode === "apps"} onclick={() => (mode = "apps")}>by app</button>
    <button class:on={mode === "two"} onclick={() => (mode = "two")}>2 colours</button>
    <span class="gap"></span>
    <button class="on">5m</button>
    {#each ["1h", "24h", "30d"] as r (r)}
      <button disabled title="Not built yet">{r}</button>
    {/each}
  </div>
  {#if width > 0}
    <svg
      {width}
      height={H}
      role="img"
      aria-label="Traffic over the last 5 minutes"
      onmousemove={move}
      onmouseleave={() => (hover = null)}
    >
      {#each [0.5, 1] as f (f)}
        <line x1="0" x2={width} y1={MID - f * D_EXT} y2={MID - f * D_EXT} class="grid" />
      {/each}
      {#if mode === "apps"}
        {#each stacks as s, k (k)}
          <polygon points={s.down} fill={s.fill} fill-opacity="0.8" />
          <polygon points={s.up} fill={s.fill} fill-opacity="0.42" />
        {/each}
      {:else}
        <polygon points="0,{MID} {twoDown} {width},{MID}" class="fill-in" />
        <polyline points={twoDown} class="stroke-in" />
        <polygon points="0,{MID} {twoUp} {width},{MID}" class="fill-out" />
        <polyline points={twoUp} class="stroke-out" />
      {/if}
      <line x1="0" x2={width} y1={MID} y2={MID} class="axis" />
      <text x="6" y={MID - D_EXT + 10} class="lbl">▲ {rate(graph.downMax)}</text>
      <text x="6" y={MID + U_EXT - 2} class="lbl">▼ {rate(graph.upMax)}</text>
      {#each TICKS as t, i (t)}
        <text x={(i / (TICKS.length - 1)) * (width - 40) + 6} y={H - 5} class="tick">{t}</text>
      {/each}
      {#if hover !== null}
        <line x1={x(hover)} x2={x(hover)} y1="24" y2={H - FOOT} class="cursor" />
      {/if}
    </svg>
  {/if}
  {#if hover !== null}
    <div class="tip" style:left="{tipX}px">
      <div class="when num">
        {clock(graph.start + hover * 1000)} · ↓ {rate(graph.down[hover])} · ↑ {rate(graph.up[hover])}
      </div>
      {#if mode === "apps"}
        {#each graph.bands as b (b.name)}
          <div class="row">
            <i style:background={paint(b.color)}></i>
            <span class="ell">{b.name}</span>
            <span class="num in">{rate(b.down[hover])}</span>
            <span class="num out">{rate(b.up[hover])}</span>
          </div>
        {/each}
      {/if}
    </div>
  {/if}
</div>

<style>
  .graphwrap {
    position: relative;
    padding-top: 28px;
    border-bottom: 1px solid var(--line);
    flex: none;
  }
  svg {
    display: block;
  }
  .legend {
    position: absolute;
    top: 8px;
    left: 14px;
    display: flex;
    gap: 14px;
    font: 11px var(--mono);
    color: var(--mute);
  }
  .legend span {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .legend i,
  .row i {
    width: 8px;
    height: 8px;
    border-radius: 2px;
    display: inline-block;
    flex: none;
  }
  .bar {
    position: absolute;
    top: 8px;
    right: 12px;
    display: flex;
    gap: 2px;
  }
  .bar button {
    font: 11px var(--mono);
    padding: 2px 7px;
    color: var(--mute);
    background: none;
    border: 0;
    border-radius: 4px;
  }
  .bar button.on {
    background: var(--raise);
    color: var(--ink);
  }
  .bar button:disabled {
    color: var(--dim);
  }
  .bar .gap {
    width: 10px;
  }
  .grid {
    stroke: var(--raise);
  }
  .axis {
    stroke: var(--dim);
  }
  .lbl,
  .tick {
    font: 10px var(--mono);
  }
  .lbl {
    fill: var(--mute);
  }
  .tick {
    fill: var(--dim);
  }
  .cursor {
    stroke: var(--kraft);
    stroke-dasharray: 2 3;
  }
  .fill-in {
    fill: var(--in);
    fill-opacity: 0.35;
  }
  .fill-out {
    fill: var(--out);
    fill-opacity: 0.3;
  }
  .stroke-in,
  .stroke-out {
    fill: none;
    stroke-width: 1.3;
  }
  .stroke-in {
    stroke: var(--in);
  }
  .stroke-out {
    stroke: var(--out);
  }
  .tip {
    position: absolute;
    top: 52px;
    width: 232px;
    padding: 7px 10px;
    border-radius: 5px;
    background: var(--panel);
    border: 1px solid var(--line);
    pointer-events: none;
    font-size: 11px;
  }
  .when {
    color: var(--mute);
    font-size: 10px;
    margin-bottom: 3px;
    white-space: nowrap;
  }
  .row {
    display: grid;
    grid-template-columns: 8px minmax(0, 1fr) auto auto;
    gap: 6px;
    align-items: center;
  }
  .row .num {
    font-size: 10px;
  }
</style>
