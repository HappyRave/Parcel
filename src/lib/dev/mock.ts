import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppRate, Commands, HostRate, ServiceState, Tick } from "$lib/api";

/** A host's fixed fields, plus its share of the app's traffic. */
type MockHost = Omit<HostRate, "down" | "up" | "packetsDown" | "packetsUp"> & { share: number };

const host = (
  name: string,
  service: string | null,
  domain: string | null,
  country: string | null,
  city: string | null,
  asn: number | null,
  network: string | null,
  share: number,
): MockHost => ({ host: name, service, domain, country, city, asn, network, share });

const APPS: { path: string; name: string; share: number; hosts: MockHost[] }[] = [
  {
    path: "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    name: "Google Chrome",
    share: 0.7,
    hosts: [
      host("rr3---sn-5hne6nzk.googlevideo.com", "YouTube", "googlevideo.com", "BE", "Brussels", 15169, "Google LLC", 0.8),
      host("www.youtube.com", "YouTube", "youtube.com", "BE", "Saint-Ghislain", 15169, "Google LLC", 0.15),
      host("github.com", "GitHub", "github.com", "NL", "Amsterdam", 36459, "GitHub, Inc.", 0.04),
      host("cdn.example.org", "example.org", "example.org", "US", "Ashburn", 15133, "Edgecast Inc.", 0.01),
    ],
  },
  {
    path: "C:\\Program Files (x86)\\Steam\\steam.exe",
    name: "Steam",
    share: 0.2,
    hosts: [host("cache5-ams1.steamcontent.com", "Steam", "steamcontent.com", "NL", "Amsterdam", 32590, "Valve Corporation", 1)],
  },
  {
    path: "C:\\Users\\me\\AppData\\Local\\Discord\\app-1.0\\Discord.exe",
    name: "Discord",
    share: 0.07,
    hosts: [
      host("gateway.discord.gg", "Discord", "discord.gg", "US", "San Jose", 13335, "Cloudflare, Inc.", 0.6),
      host("162.159.135.234", "Cloudflare", null, "US", "San Francisco", 13335, "Cloudflare, Inc.", 0.4),
    ],
  },
  {
    path: "",
    name: "Unknown app",
    share: 0.03,
    hosts: [host("239.255.255.250", null, null, null, null, null, null, 1)],
  },
];

/** Dev-only fake backend: `pnpm dev`, then open `/?mock`. Ticks wobbly traffic every second. */
export function installMock() {
  const state: ServiceState = { state: "connected", service: "mock service" };
  const commands: { [K in keyof Commands]: Commands[K] } = {
    service_state: () => state,
    // About 1 MB/s down and 40 KB/s up averaged over the range.
    usage: ({ from, to }) => traffic(((to - from) / 1000) * 1_000_000, ((to - from) / 1000) * 40_000),
  };
  mockIPC(
    (cmd, args) => {
      if (cmd.startsWith("plugin:opener|")) {
        return null;
      }
      const handler = commands[cmd as keyof Commands] as ((a: unknown) => unknown) | undefined;
      if (!handler) {
        throw new Error(`mock: no command ${cmd}`);
      }
      return handler(args);
    },
    { shouldMockEvents: true },
  );

  let t = 0;
  setInterval(() => {
    t += 1;
    const down = Math.round(2_000_000 + 1_500_000 * Math.sin(t / 5));
    const up = Math.round(80_000 + 40_000 * Math.cos(t / 3));
    const tick: Tick = { at: Math.floor(Date.now() / 1000) * 1000, down, up, apps: traffic(down, up) };
    void emit("tick", tick);
  }, 1000);
}

/** The mock apps sharing `down` / `up` bytes. */
function traffic(down: number, up: number): AppRate[] {
  return APPS.map((a) => ({
    path: a.path,
    name: a.name,
    down: Math.round(down * a.share),
    up: Math.round(up * a.share),
    packetsDown: Math.round((down * a.share) / 1400),
    packetsUp: Math.round((up * a.share) / 300),
    hosts: a.hosts.map(({ share, ...h }) => ({
      ...h,
      down: Math.round(down * a.share * share),
      up: Math.round(up * a.share * share),
      packetsDown: Math.round((down * a.share * share) / 1400),
      packetsUp: Math.round((up * a.share * share) / 300),
    })),
  }));
}
