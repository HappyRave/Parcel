import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";

/** Traffic of one app over a span (History's totals), in bytes. */
export interface AppRate {
  /** Executable path, the app's identity; "" for traffic with no known owner. */
  path: string;
  name: string;
  down: number;
  up: number;
  /** Zero without Npcap. */
  packetsDown: number;
  packetsUp: number;
  /** Busiest first. */
  hosts: HostRate[];
}

export interface HostRate {
  /** Hostname from DNS or SNI, else the IP address. */
  host: string;
  /** Brand ("YouTube"), else the registered domain, else (bare IP) the network's owner. */
  service: string | null;
  /** Registered domain, for the favicon. */
  domain: string | null;
  /** ISO 3166-1 alpha-2. */
  country: string | null;
  city: string | null;
  asn: number | null;
  /** AS organization, e.g. "Google LLC". */
  network: string | null;
  down: number;
  up: number;
  packetsDown: number;
  packetsUp: number;
}

/** Everything the Live tab draws, sent by the Rust side every second. */
export interface LiveView {
  kpis: Kpis;
  graph: Graph;
  /** Busiest first, over the last 5 minutes. */
  apps: AppRow[];
  /** The app whose hosts are listed. */
  selected: string | null;
  hosts: HostRow[];
}

export interface Kpis {
  down: number;
  up: number;
  todayDown: number;
  todayUp: number;
  /** null without Npcap. */
  packetsPerMin: number | null;
  appsNow: number;
  hostsNow: number;
}

export interface Graph {
  /** Time of the first one-second slot, ms since the epoch. */
  start: number;
  downMax: number;
  upMax: number;
  down: number[];
  up: number[];
  /** Stacked from the axis outwards: the top apps, then "other". */
  bands: Band[];
}

export interface Band {
  name: string;
  /** Palette index (`--c0`…`--c9`); null is grey. */
  color: number | null;
  down: number[];
  up: number[];
}

export interface AppRow {
  path: string;
  name: string;
  /** e.g. "chrome.exe"; empty for System and stand-ins. */
  exe: string;
  color: number | null;
  down: number;
  up: number;
}

export interface HostRow {
  host: string;
  color: number | null;
  service: string | null;
  domain: string | null;
  country: string | null;
  city: string | null;
  asn: number | null;
  network: string | null;
  packets: number;
  down: number;
  up: number;
  /** Bytes per second over the last 60 s. */
  spark: number[];
}

export interface Capture {
  npcap: boolean;
  etw: boolean;
}

export type ServiceState =
  | { state: "connecting" }
  | { state: "connected"; service: string; capture: Capture }
  | { state: "offline"; reason: string };

export interface Commands {
  service_state: () => ServiceState;
  usage: (args: { from: number; to: number }) => AppRate[];
  select_app: (args: { path: string | null }) => void;
}

/** The CSS colour for a palette index. */
export const paint = (color: number | null) => (color === null ? "var(--other)" : `var(--c${color})`);

export const serviceState = () => invoke<ServiceState>("service_state");

/** History totals per app and host between two times (ms since the epoch), busiest first. */
export const usage = (from: number, to: number) => invoke<AppRate[]>("usage", { from, to });

/** Picks the app whose hosts the Live tab lists; null follows the busiest. */
export const selectApp = (path: string | null) => invoke<void>("select_app", { path });

/** Opens DB-IP's site, the attribution its licence asks for. */
export const openDbIp = () => openUrl("https://db-ip.com");

export const onLive = (cb: (view: LiveView) => void): Promise<UnlistenFn> =>
  listen<LiveView>("live", (e) => cb(e.payload));

export const onService = (cb: (state: ServiceState) => void): Promise<UnlistenFn> =>
  listen<ServiceState>("service", (e) => cb(e.payload));
