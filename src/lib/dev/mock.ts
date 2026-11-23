import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppRate, Commands, ServiceState, Tick } from "$lib/api";

/** Host, service, registered domain, share of the app's traffic. */
type MockHost = [string, string | null, string | null, number];

const APPS: { path: string; name: string; share: number; hosts: MockHost[] }[] = [
  {
    path: "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    name: "Google Chrome",
    share: 0.7,
    hosts: [
      ["rr3---sn-5hne6nzk.googlevideo.com", "YouTube", "googlevideo.com", 0.8],
      ["www.youtube.com", "YouTube", "youtube.com", 0.15],
      ["github.com", "GitHub", "github.com", 0.04],
      ["cdn.example.org", "example.org", "example.org", 0.01],
    ],
  },
  {
    path: "C:\\Program Files (x86)\\Steam\\steam.exe",
    name: "Steam",
    share: 0.2,
    hosts: [["cache5-ams1.steamcontent.com", "Steam", "steamcontent.com", 1]],
  },
  {
    path: "C:\\Users\\me\\AppData\\Local\\Discord\\app-1.0\\Discord.exe",
    name: "Discord",
    share: 0.07,
    hosts: [
      ["gateway.discord.gg", "Discord", "discord.gg", 0.6],
      ["162.159.135.234", null, null, 0.4],
    ],
  },
  { path: "", name: "Unknown app", share: 0.03, hosts: [["239.255.255.250", null, null, 1]] },
];

/** Dev-only fake backend: `pnpm dev`, then open `/?mock`. Ticks wobbly traffic every second. */
export function installMock() {
  const state: ServiceState = { state: "connected", service: "mock service" };
  const commands: { [K in keyof Commands]: Commands[K] } = {
    service_state: () => state,
  };
  mockIPC(
    (cmd) => {
      const handler = commands[cmd as keyof Commands];
      if (!handler) {
        throw new Error(`mock: no command ${cmd}`);
      }
      return handler();
    },
    { shouldMockEvents: true },
  );

  let t = 0;
  setInterval(() => {
    t += 1;
    const down = Math.round(2_000_000 + 1_500_000 * Math.sin(t / 5));
    const up = Math.round(80_000 + 40_000 * Math.cos(t / 3));
    const apps: AppRate[] = APPS.map((a) => ({
      path: a.path,
      name: a.name,
      down: Math.round(down * a.share),
      up: Math.round(up * a.share),
      packetsDown: Math.round((down * a.share) / 1400),
      packetsUp: Math.round((up * a.share) / 300),
      hosts: a.hosts.map(([host, service, domain, share]) => ({
        host,
        service,
        domain,
        down: Math.round(down * a.share * share),
        up: Math.round(up * a.share * share),
        packetsDown: Math.round((down * a.share * share) / 1400),
        packetsUp: Math.round((up * a.share * share) / 300),
      })),
    }));
    const tick: Tick = { at: Math.floor(Date.now() / 1000) * 1000, down, up, apps };
    void emit("tick", tick);
  }, 1000);
}
