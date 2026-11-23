<script lang="ts" generics="Tab extends string">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import type { ServiceState } from "$lib/api";

  let {
    tabs,
    tab = $bindable(),
    service,
  }: { tabs: readonly Tab[]; tab: Tab; service: ServiceState } = $props();

  type Pill = { label: string; level: "ok" | "warn" | "bad"; title: string };
  const pills = $derived.by((): Pill[] => {
    if (service.state === "offline") {
      return [{ label: "service", level: "bad", title: service.reason }];
    }
    if (service.state === "connecting") {
      return [{ label: "service", level: "warn", title: "connecting" }];
    }
    const { npcap, etw } = service.capture;
    return [
      { label: "npcap", level: npcap ? "ok" : "warn", title: npcap ? "capturing packets" : "not running" },
      { label: "etw", level: etw ? "ok" : "warn", title: etw ? "naming apps" : "not running" },
      { label: "service", level: "ok", title: service.service },
    ];
  });

  // Looked up on click, not at creation: there is no Tauri window in `pnpm dev`'s browser.
  const win = () => getCurrentWindow();
</script>

<header class="title" data-tauri-drag-region>
  <div class="brand" data-tauri-drag-region>
    <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true">
      <path d="M8 1.5 14 4.5v7L8 14.5 2 11.5v-7z" fill="none" stroke="#c8955a" stroke-width="1.4" stroke-linejoin="round" />
      <path d="M2 4.5 8 7.5l6-3M8 7.5v7" fill="none" stroke="#c8955a" stroke-width="1.4" stroke-linejoin="round" />
      <path d="M5 3l6 3" stroke="var(--ink)" stroke-width="1.2" />
    </svg>
    parcel
  </div>
  <nav class="tabs">
    {#each tabs as t (t)}
      <button class:on={t === tab} onclick={() => (tab = t)}>{t}</button>
    {/each}
  </nav>
  <div class="status" data-tauri-drag-region>
    {#each pills as p (p.label)}
      <span class="pill {p.level}" title={p.title}><i></i>{p.label}</span>
    {/each}
  </div>
  <div class="spacer" data-tauri-drag-region></div>
  <button class="wbtn" aria-label="Minimize" onclick={() => win().minimize()}>&#8211;</button>
  <button class="wbtn" aria-label="Maximize" onclick={() => win().toggleMaximize()}>&#9633;</button>
  <button class="wbtn close" aria-label="Close" onclick={() => win().close()}>&#10005;</button>
</header>

<style>
  .title {
    height: 34px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding-left: 12px;
    border-bottom: 1px solid var(--line);
    background: var(--title);
    user-select: none;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 7px;
    font-weight: 600;
    letter-spacing: 0.3px;
  }
  .tabs {
    display: flex;
    gap: 2px;
    margin-left: 18px;
  }
  .tabs button {
    font: inherit;
    color: var(--mute);
    background: none;
    border: 0;
    padding: 5px 10px;
    border-radius: 5px;
    cursor: default;
  }
  .tabs button.on {
    color: var(--ink);
    background: var(--raise);
  }
  .status {
    display: flex;
    gap: 6px;
    margin-left: 8px;
  }
  .pill {
    font: 11px var(--mono);
    color: var(--mute);
    padding: 1px 7px;
    border: 1px solid var(--line);
    border-radius: 10px;
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .pill i {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok);
  }
  .pill.warn i {
    background: var(--warn);
  }
  .pill.bad i {
    background: var(--bad);
  }
  .spacer {
    flex: 1;
    align-self: stretch;
  }
  .wbtn {
    width: 44px;
    height: 34px;
    display: grid;
    place-items: center;
    color: var(--mute);
    background: none;
    border: 0;
    font: 13px var(--ui);
  }
  .wbtn:hover {
    background: var(--raise);
    color: var(--ink);
  }
  .wbtn.close:hover {
    background: #c42b1c;
    color: #fff;
  }
</style>
