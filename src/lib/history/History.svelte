<script lang="ts">
  import { onMount } from "svelte";
  import { usage, type AppRate } from "$lib/api";
  import { bytes } from "$lib/format";
  import Apps from "$lib/live/Apps.svelte";

  const DAY = 86_400_000;
  /** R4 Y1 ranges. */
  const ranges = [
    { label: "30 days", span: 30 * DAY },
    { label: "12 months", span: 365 * DAY },
    { label: "All", span: null },
  ] as const;

  let range = $state<(typeof ranges)[number]>(ranges[0]);
  let apps = $state<AppRate[]>([]);
  let error = $state<string | null>(null);

  async function load() {
    const now = Date.now();
    try {
      apps = await usage(range.span === null ? 0 : now - range.span, now);
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    void range;
    load();
  });

  onMount(() => {
    // History grows by the minute.
    const timer = setInterval(load, 60_000);
    return () => clearInterval(timer);
  });

  const total = $derived(apps.reduce((s, a) => [s[0] + a.down, s[1] + a.up], [0, 0]));
</script>

<section class="head">
  <div class="seg">
    {#each ranges as r (r.label)}
      <button class:on={r === range} onclick={() => (range = r)}>{r.label}</button>
    {/each}
  </div>
  <span class="num in">↓ {bytes(total[0])}</span>
  <span class="num out">↑ {bytes(total[1])}</span>
  {#if error}<span class="err">{error}</span>{/if}
</section>
<Apps {apps} amount={bytes} empty="Nothing recorded in this range yet." />

<style>
  .head {
    display: flex;
    align-items: center;
    gap: 16px;
  }
  .seg {
    display: flex;
    border: 1px solid var(--line);
    border-radius: 5px;
    overflow: hidden;
  }
  .seg button {
    font: 12px var(--ui);
    color: var(--mute);
    background: none;
    border: 0;
    padding: 4px 10px;
    cursor: pointer;
  }
  .seg button.on {
    background: var(--raise);
    color: var(--ink);
  }
  .num {
    font-size: 15px;
  }
  .in {
    color: var(--in);
  }
  .out {
    color: var(--out);
  }
  .err {
    color: var(--bad);
  }
</style>
