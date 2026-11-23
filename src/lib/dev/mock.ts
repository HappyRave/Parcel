import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppRate, Commands, HostRow, LiveView, ServiceState } from "$lib/api";

const WINDOW = 300;

type MockHost = Omit<HostRow, "down" | "up" | "packets" | "spark"> & { share: number };

const host = (
  name: string,
  service: string | null,
  country: string | null,
  city: string | null,
  asn: number | null,
  network: string | null,
  color: number | null,
  share: number,
): MockHost => ({ host: name, service, domain: null, country, city, asn, network, color, share });

/** Fake apps: a share of the download / upload, a palette colour, hosts. */
const APPS = [
  {
    path: "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    name: "Google Chrome",
    color: 0,
    down: 0.62,
    up: 0.45,
    hosts: [
      host("rr3---sn-5hne6nzk.googlevideo.com", "YouTube", "BE", "Brussels", 15169, "Google LLC", 2, 0.7),
      host("www.youtube.com", "YouTube", "BE", "Saint-Ghislain", 15169, "Google LLC", 2, 0.12),
      host("github.githubassets.com", "GitHub", "NL", "Amsterdam", 54113, "Fastly, Inc.", 1, 0.1),
      host("www.google.com", "Google", "BE", "Saint-Ghislain", 15169, "Google LLC", 0, 0.05),
      host("151.101.65.140", "Fastly", "US", "San Francisco", 54113, "Fastly, Inc.", 5, 0.03),
    ],
  },
  {
    path: "C:\\Program Files (x86)\\Steam\\steam.exe",
    name: "Steam",
    color: 1,
    down: 0.18,
    up: 0.05,
    hosts: [host("cache4-ams1.steamcontent.com", "Steam", "NL", "Amsterdam", 32590, "Valve Corporation", 1, 1)],
  },
  {
    path: "C:\\Users\\me\\AppData\\Local\\AnthropicClaude\\claude.exe",
    name: "Claude",
    color: 2,
    down: 0.09,
    up: 0.25,
    hosts: [host("api.anthropic.com", "Anthropic", "US", "Ashburn", 399358, "Anthropic, PBC", 2, 1)],
  },
  {
    path: "C:\\Users\\me\\AppData\\Local\\Discord\\app-1.0\\Discord.exe",
    name: "Discord",
    color: 3,
    down: 0.06,
    up: 0.1,
    hosts: [host("gateway.discord.gg", "Discord", "US", "San Jose", 13335, "Cloudflare, Inc.", 3, 1)],
  },
  {
    path: "C:\\Program Files\\Spotify\\Spotify.exe",
    name: "Spotify",
    color: 4,
    down: 0.03,
    up: 0.07,
    hosts: [host("audio-ak.spotifycdn.com", "Spotify", "SE", "Stockholm", 16625, "Akamai Technologies", 4, 1)],
  },
  {
    path: "C:\\Program Files\\Parcel\\parcel-service.exe",
    name: "Parcel",
    color: 5,
    down: 0.02,
    up: 0.08,
    hosts: [host("www.youtube.com", "YouTube", "BE", "Saint-Ghislain", 15169, "Google LLC", 2, 1)],
  },
];

/** Dev-only fake backend: `pnpm dev`, then open `/?mock`. Ticks a fake Live view every second. */
export function installMock() {
  const state: ServiceState = {
    state: "connected",
    service: "mock service",
    capture: { npcap: true, etw: true },
  };
  let selected: string | null = null;
  // Per-second totals over the window, seeded so the graph starts full.
  let seed = 7;
  const rnd = () => (seed = (seed * 9301 + 49297) % 233280) / 233280;
  let level = 0.4;
  const next = () => {
    level = Math.max(0.08, Math.min(1, level + (rnd() - 0.5) * 0.35));
    const burst = rnd() > 0.8 ? 0.3 + rnd() * 0.6 : 0;
    return { d: Math.min(1, level * 0.6 + burst) * 5_000_000, u: (0.05 + rnd() * 0.12) * 900_000 };
  };
  const series = Array.from({ length: WINDOW }, next);

  const view = (): LiveView => {
    const at = Math.floor(Date.now() / 1000) * 1000;
    const wobble = (k: number, i: number) => 1 + 0.35 * Math.sin(i / 9 + k * 2);
    const per = APPS.map((a, k) => ({
      down: series.map((s, i) => s.d * a.down * wobble(k, i)),
      up: series.map((s, i) => s.u * a.up * wobble(k, i)),
    }));
    const sum = (v: number[]) => v.reduce((x, y) => x + y, 0);
    const rows = APPS.map((a, k) => ({
      path: a.path,
      name: a.name,
      exe: a.path.split("\\").pop() ?? "",
      color: a.color,
      down: Math.round(sum(per[k].down)),
      up: Math.round(sum(per[k].up)),
    })).sort((x, y) => y.down + y.up - (x.down + x.up));
    const top = rows.slice(0, 4).map((r) => APPS.findIndex((a) => a.path === r.path));
    const rest = APPS.map((_, k) => k).filter((k) => !top.includes(k));
    const fold = (pick: "down" | "up") => series.map((_, i) => rest.reduce((s, k) => s + per[k][pick][i], 0));
    const bands = [
      ...top.map((k) => ({ name: APPS[k].name, color: APPS[k].color, down: per[k].down, up: per[k].up })),
      { name: "other", color: null, down: fold("down"), up: fold("up") },
    ];
    const pick = APPS.find((a) => a.path === selected) ?? APPS.find((a) => a.path === rows[0].path)!;
    const k = APPS.indexOf(pick);
    const hosts = pick.hosts.map(({ share, ...h }) => ({
      ...h,
      down: Math.round(sum(per[k].down) * share),
      up: Math.round(sum(per[k].up) * share),
      packets: Math.round((sum(per[k].down) * share) / 1300),
      spark: per[k].down.slice(-60).map((v, i) => Math.round(v * share * wobble(3, i * 3))),
    }));
    const last = series[WINDOW - 1];
    return {
      kpis: {
        down: Math.round(last.d),
        up: Math.round(last.u),
        todayDown: 26_200_000_000,
        todayUp: 1_410_000_000,
        packetsPerMin: 30_072,
        appsNow: APPS.length,
        hostsNow: 42,
      },
      graph: {
        start: at - (WINDOW - 1) * 1000,
        downMax: Math.max(...series.map((s) => s.d)),
        upMax: Math.max(...series.map((s) => s.u)),
        down: series.map((s) => s.d),
        up: series.map((s) => s.u),
        bands,
      },
      apps: rows,
      selected: pick.path,
      hosts,
    };
  };

  const commands: { [K in keyof Commands]: Commands[K] } = {
    service_state: () => state,
    usage: ({ from, to }) => traffic(((to - from) / 1000) * 1_000_000, ((to - from) / 1000) * 40_000),
    select_app: ({ path }) => {
      selected = path;
      void emit("live", view());
    },
  };
  mockIPC(
    (cmd, args) => {
      if (cmd.startsWith("plugin:opener|") || cmd.startsWith("plugin:window|")) {
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

  setInterval(() => {
    series.shift();
    series.push(next());
    void emit("live", view());
  }, 1000);
}

/** History totals: the mock apps sharing `down` / `up` bytes. */
function traffic(down: number, up: number): AppRate[] {
  return APPS.map((a) => ({
    path: a.path,
    name: a.name,
    down: Math.round(down * a.down),
    up: Math.round(up * a.up),
    packetsDown: Math.round((down * a.down) / 1400),
    packetsUp: Math.round((up * a.up) / 300),
    hosts: a.hosts.map(({ share, color: _color, ...h }) => ({
      ...h,
      down: Math.round(down * a.down * share),
      up: Math.round(up * a.up * share),
      packetsDown: Math.round((down * a.down * share) / 1400),
      packetsUp: Math.round((up * a.up * share) / 300),
    })),
  }));
}
