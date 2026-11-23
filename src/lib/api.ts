import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";

/** Traffic over one second, in bytes. `at` is the end of that second, ms since the epoch. */
export interface Tick {
  at: number;
  down: number;
  up: number;
  /** Busiest first. */
  apps: AppRate[];
}

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
  /** Brand ("YouTube"), else the registered domain; null for a bare IP. */
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

export type ServiceState =
  | { state: "connecting" }
  | { state: "connected"; service: string }
  | { state: "offline"; reason: string };

export interface Commands {
  service_state: () => ServiceState;
}

export const serviceState = () => invoke<ServiceState>("service_state");

/** Opens DB-IP's site, the attribution its licence asks for. */
export const openDbIp = () => openUrl("https://db-ip.com");

export const onTick = (cb: (tick: Tick) => void): Promise<UnlistenFn> =>
  listen<Tick>("tick", (e) => cb(e.payload));

export const onService = (cb: (state: ServiceState) => void): Promise<UnlistenFn> =>
  listen<ServiceState>("service", (e) => cb(e.payload));
