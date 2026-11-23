<script lang="ts">
  import { onMount } from "svelte";
  import {
    onService,
    onTick,
    openDbIp,
    serviceState,
    type ServiceState,
    type Tick,
  } from "$lib/api";
  import { rate } from "$lib/format";
  import History from "$lib/history/History.svelte";
  import Apps from "$lib/live/Apps.svelte";

  const tabs = ["Live", "Map", "Flow", "History"] as const;
  type Tab = (typeof tabs)[number];

  let tab = $state<Tab>("Live");
  let service = $state<ServiceState>({ state: "connecting" });
  let last = $state<Tick | null>(null);

  onMount(() => {
    const stops = [onService((s) => (service = s)), onTick((t) => (last = t))];
    serviceState().then((s) => (service = s));
    return () => stops.forEach((p) => p.then((stop) => stop()));
  });
</script>

<div class="app">
  <header class="title">
    <span class="brand">Parcel</span>
    <nav>
      {#each tabs as t (t)}
        <button class:on={tab === t} onclick={() => (tab = t)}>{t}</button>
      {/each}
    </nav>
    <span class="link {service.state}" title={service.state === "offline" ? service.reason : ""}>
      {service.state === "connected" ? service.service : service.state}
    </span>
  </header>

  {#if service.state === "offline"}
    <div class="banner">Not recording: {service.reason}.</div>
  {/if}

  <main>
    {#if tab === "Live"}
      <section class="kpis">
        <div class="kpi"><span>Download</span><b class="num in">↓ {rate(last?.down ?? 0)}</b></div>
        <div class="kpi"><span>Upload</span><b class="num out">↑ {rate(last?.up ?? 0)}</b></div>
      </section>
      <Apps apps={last?.apps ?? []} />
      <footer>
        <button class="credit" onclick={() => openDbIp()}>IP Geolocation by DB-IP</button>
      </footer>
    {:else if tab === "History"}
      <History />
    {:else}
      <p class="todo">{tab} isn't built yet.</p>
    {/if}
  </main>
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .title {
    height: 34px;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 0 12px;
    background: var(--title);
    border-bottom: 1px solid var(--line);
  }
  .brand {
    font-weight: 600;
    color: var(--kraft);
  }
  nav {
    display: flex;
    gap: 2px;
  }
  nav button {
    font: inherit;
    color: var(--mute);
    background: none;
    border: 0;
    padding: 4px 10px;
    border-radius: 5px;
    cursor: pointer;
  }
  nav button.on {
    color: var(--ink);
    background: var(--raise);
  }
  .link {
    margin-left: auto;
    font: 11px var(--mono);
    color: var(--mute);
  }
  .link.offline {
    color: var(--bad);
  }
  .banner {
    padding: 6px 12px;
    background: var(--raise);
    border-bottom: 1px solid var(--line);
    color: var(--warn);
  }
  main {
    flex: 1;
    padding: 16px;
  }
  .kpis {
    display: flex;
    gap: 12px;
  }
  .kpi {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 14px;
    min-width: 160px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 8px;
  }
  .kpi span {
    font-size: 11px;
    color: var(--mute);
  }
  .kpi b {
    font-size: 18px;
    font-weight: 500;
  }
  .in {
    color: var(--in);
  }
  .out {
    color: var(--out);
  }
  .todo {
    color: var(--dim);
  }
  footer {
    margin-top: 8px;
    text-align: right;
  }
  .credit {
    font: 11px var(--ui);
    color: var(--dim);
    background: none;
    border: 0;
    padding: 0;
    cursor: pointer;
    text-decoration: underline;
  }
</style>
