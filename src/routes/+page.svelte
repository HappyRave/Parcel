<script lang="ts">
  import { onMount } from "svelte";
  import {
    onLive,
    onService,
    openDbIp,
    selectApp,
    serviceState,
    type LiveView,
    type ServiceState,
  } from "$lib/api";
  import TitleBar from "$lib/chrome/TitleBar.svelte";
  import History from "$lib/history/History.svelte";
  import AppList from "$lib/live/AppList.svelte";
  import HostTable from "$lib/live/HostTable.svelte";
  import Kpis from "$lib/live/Kpis.svelte";
  import LiveGraph from "$lib/live/LiveGraph.svelte";

  const tabs = ["Live", "Map", "Flow", "History"] as const;

  let tab = $state<(typeof tabs)[number]>("Live");
  let service = $state<ServiceState>({ state: "connecting" });
  let live = $state<LiveView | null>(null);

  const selectedName = $derived(live?.apps.find((a) => a.path === live?.selected)?.name ?? "");
  const npcap = $derived(service.state === "connected" && service.capture.npcap);

  onMount(() => {
    const stops = [onService((s) => (service = s)), onLive((v) => (live = v))];
    serviceState().then((s) => (service = s));
    return () => stops.forEach((p) => p.then((stop) => stop()));
  });
</script>

<div class="app">
  <TitleBar {tabs} bind:tab {service} />

  {#if service.state === "offline"}
    <div class="banner">Not recording: {service.reason}.</div>
  {/if}

  {#if tab === "Live"}
    {#if live}
      <Kpis kpis={live.kpis} />
      <LiveGraph graph={live.graph} />
      <div class="lower">
        <AppList apps={live.apps} selected={live.selected} onpick={(path) => selectApp(path)} />
        <HostTable hosts={live.hosts} app={selectedName} packets={npcap} />
      </div>
    {:else}
      <p class="todo">Waiting for the service…</p>
    {/if}
  {:else if tab === "History"}
    <div class="padded"><History /></div>
  {:else}
    <p class="todo">{tab} isn't built yet.</p>
  {/if}

  <footer>
    <button class="credit" onclick={() => openDbIp()}>IP Geolocation by DB-IP</button>
  </footer>
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .banner {
    padding: 9px 14px;
    background: color-mix(in srgb, var(--warn) 14%, var(--win));
    border-bottom: 1px solid color-mix(in srgb, var(--warn) 35%, var(--win));
    font-size: 12px;
    flex: none;
  }
  .lower {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 340px 1fr;
  }
  .padded {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 16px;
  }
  .todo {
    flex: 1;
    color: var(--dim);
    padding: 16px;
    margin: 0;
  }
  footer {
    flex: none;
    text-align: right;
    padding: 2px 12px 3px;
    border-top: 1px solid var(--line);
  }
  .credit {
    font: 10px var(--ui);
    color: var(--dim);
    background: none;
    border: 0;
    padding: 0;
    cursor: pointer;
  }
  .credit:hover {
    text-decoration: underline;
  }
</style>
