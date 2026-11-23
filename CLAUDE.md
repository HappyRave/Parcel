# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Parcel is a lightweight desktop network monitor. It shows live PC network traffic and where it goes:
per remote host, with the company/service logo, geographic origin, packet counts and data volume.
The look takes inspiration from GlassWire's mechanics (live traffic graph, per-app / per-host
breakdown) but is our own design, not a copy of its visuals.

**State (2026-11-23):** capture, attribution, naming, GeoIP, history and the Windows service
work. The service (installed, or in console mode) runs Npcap (packets, DNS and SNI names), ETW
(per-process transfers + DNS-client names, admin only) and a 1 s socket-table poll into the
engine, which sends per-second ticks by app and remote host over the pipe and records them to
SQLite; the app shows KPIs, an apps table and the selected app's hosts as service · host with
location (stamp + city) and network, plus the DB-IP credit, and a History tab (30 days / 12
months / all) with the same tables as totals (plain tables, not the lab design). Not built:
favicons, the per-second backlog for the live graph, the installer, the lab UI (graphs, Map,
Flow, tray, settings). Update this file as pieces land.

Windows service (`service.rs`, `runner.rs`): `parcel-service install` registers service `Parcel`
(automatic start, LocalSystem, restarts 5 s / 30 s / 5 min after a failure, also on an error
exit) with this exe and the argument `run`, then starts it; `uninstall` stops and deletes it.
Both need admin and say so plainly otherwise. Console and service share `runner::run`, which
stops on Ctrl+C or the Service Control Manager's stop / shutdown. The service logs to
`parcel-service.log` in the data folder, fresh each start. While installed from `target\debug`
it locks that exe: uninstall (or stop it) before rebuilding `parcel-service`. Verified live
2026-11-23: install, runs as LocalSystem with ETW on, the app connects, uninstall leaves no
service and no ETW session.

History (`store.rs`): `parcel.db` in the data folder, WAL. Tables `apps`, `hosts` (latest
metadata) and `minute` / `hour` / `day` totals keyed (t, app, host), never packets. The
`Recorder` folds ticks into the current minute and writes all three resolutions in one
transaction when the minute ends (and on shutdown); minute rows go after 24 h, hour rows after
30 days, day rows never. Day keys are local calendar days (Claude's pick: "today" follows the
user's clock, the UTC offset taken at write time). `Store::usage` reads the finest table still
covering the range's start, so edges snap to that resolution. The app asks through the pipe
(`Request::Usage` → `Reply::Usage`, matched by id between ticks); the service answers from a
read-only connection on a blocking thread.

Naming (`naming.rs`): hostname → registered domain (`psl`) → `brands.json` on the registered
domain, else on the public suffix (so `x.github.io` is GitHub) → service = brand, else the
registered domain. A bare IP's service is its AS organization without legal suffixes
(`geo::org_name`: "Fastly, Inc." → "Fastly"). `brands.json` (brand → domains, 200+ domains) is
Claude's draft, 2026-11-23, not yet reviewed by the owner; a test keeps every entry a lowercase
registered domain or public suffix, listed once.

GeoIP (`geo.rs`, `parcel-service/src/dbip.rs`): DB-IP City Lite + ASN Lite, memory-mapped, in
the data folder (`PARCEL_DATA`, else `%ProgramData%\Parcel`) as `dbip-{city,asn}-lite-YYYY-MM.mmdb`.
The service checks daily, downloads the current month (last month's if not published yet) from
`https://download.db-ip.com/free/<name>.gz`, validates it, swaps it into the engine's
`Arc<RwLock<Geo>>` and deletes older months. Files are never rewritten in place (that is what
makes the mmap safe). ASN fields are `autonomous_system_number` / `_organization` (checked
against the real file; DB-IP's docs don't list them). The ignored test `real_databases_answer`
runs against real files: `PARCEL_DATA=C:\ProgramData\Parcel cargo test -p parcel-core -- --ignored`.
The credit "IP Geolocation by DB-IP" links to db-ip.com via `tauri-plugin-opener` (scoped to
that URL). Lite data is coarse: anycast IPs land anywhere (1.1.1.1 → Sydney), and city names
can carry a district in parentheses.

Attribution: a flow's owner is keyed by (protocol, local port), learned from ETW transfers and
the socket-table poll; pid 0 (TIME_WAIT sockets) never replaces an owner. A process is named
when its pid is first seen, since short-lived ones are gone by the tick. Bytes with no owner yet
are held up to 2 ticks and given to their owner if one turns up, else to "Unknown app"
(decided 2026-11-23, the owner picked "Hold back up to 2 s": ETW delivers in ~1 s batches, often
after Npcap's packets; holding took a short-download test from 5.7 of 15 MB unknown to none).
The KPI totals are not held. ETW's Kernel-Process start events (`Event::ProcessStart`, image
mapped from `\Device\HarddiskVolumeN\...` to a drive path by `ntpath.rs`) give each pid its exe
path, so a process that exits before ETW's batch names it is still named; a start event also
drops the cached app for a reused pid. Verified 2026-11-23: five 0.25 s curl downloads, 15.5 MB,
all attributed to curl. Without ETW (no admin) an exited process is still "Process <pid>".
Apps are keyed by exe path (decided 2026-11-23, the owner: "Keep path identity", so two exes
with one description stay two rows) and named by the exe's `FileDescription` (what Task Manager shows),
else its file name (Claude's pick; curl shows as "The curl executable"). pids 0 and 4 are
"System", the service itself "Parcel", an unreadable pid "Process <pid>". Hosts take the SNI
name, else the DNS name, else the IP. Without admin, ETW is off and ownership comes from the
poll alone, so short-lived flows can land in "Unknown app". ETW bytes (no-Npcap mode) are not
filtered to hardware adapters yet, so they include VPN / WSL / Hyper-V traffic.

## Commands

```powershell
cargo run -p parcel-service -- console   # the service in this terminal (pnpm service); admin for ETW
target\debug\parcel-service.exe install   # admin: register + start the Parcel service; `uninstall` removes it
cargo run -p parcel-service -- console --record events.jsonl   # also dump every Event (holds real hostnames: never commit one)
pnpm tauri dev                           # the app, unelevated, connects to \\.\pipe\parcel
pnpm dev                                 # UI only in a browser; open /?mock for the fake backend
cargo test --workspace; pnpm test; pnpm check
cargo fmt --all; cargo clippy --workspace --all-targets
```

## Layout

```
crates/parcel-core      portable logic: event.rs (the Event stream), capture.rs (CaptureSource),
                        engine.rs (byte rule, owners, names, per-app / per-host ticks, run loop),
                        packet.rs (Ethernet frame → Packet, DNS reply / TLS SNI → Hostname),
                        process.rs (App, the Processes trait), geo.rs (DB-IP lookup),
                        store.rs (SQLite history, Recorder),
                        naming.rs + brands.json
                        (hostname → service); tests/replay.rs replays
                        tests/fixtures/*.jsonl (hand-written, documentation IPs only)
crates/parcel-ipc       messages.rs (wire types, PROTOCOL), frame.rs (u32 LE length + JSON),
                        pipe.rs (Windows server with a request Handler / Connection + Requester;
                        SDDL: SYSTEM, admins, interactive users)
                        feature `pipe` (default); parcel-core takes only the messages
crates/parcel-service   binary; main.rs parses console / install / uninstall / run, runner.rs
                        starts the sources, engine thread and pipe server until told to stop,
                        service.rs talks to the Service Control Manager, data.rs is the data
                        folder. capture/npcap.rs (one thread per hardware adapter, adapters.rs
                        matches them by GUID), capture/etw.rs (session `Parcel-KernelNetwork`:
                        Kernel-Network, DNS-Client, Kernel-Process), ntpath.rs (device → drive
                        paths),
                        capture/ports.rs (socket-table poll), process.rs (pid → exe path + name),
                        dbip.rs (monthly DB-IP download), history.rs (answers
                        Request::Usage).
                        build.rs makes wpcap.lib from lib/wpcap.def with lib.exe (needs the VS C++
                        build tools) and delay-loads wpcap.dll
src-tauri               keeps one pipe Connection, re-emits `tick` and `service` events,
                        commands `service_state`, `usage` (forwarded as a pipe request)
src/                    lib/api.ts wraps invoke/listen; lib/dev/mock.ts; routes/+page.svelte;
                        lib/live/Apps.svelte (apps + selected app's hosts, rates or totals),
                        lib/history/History.svelte (ranges + totals)
spikes/                 throwaway research code, not in the workspace
```

The app icon (`src-tauri/icons/app-icon.svg`, a tied parcel) is a placeholder, never given a design
round. The identifier `com.happyrave.parcel` copies gitgud's.

## Stack (decided)

- Tauri 2 shell, Rust backend, SvelteKit frontend (Svelte 5 runes, `adapter-static`).
- Rust/Tauri/Svelte rules come from `~/.claude/rules/rust.md` (imported by the untracked
  `CLAUDE.local.md`). The main one: **all logic lives in Rust**: capture, aggregation, lookups and
  state. Svelte only calls commands, listens to events and renders.
- Sibling project `../gitgud` uses the same stack. Reuse its proven setup (workspace with a
  Tauri-free `crates/<name>-core`, thin `src-tauri`, `src/lib/api.ts` wrapping `invoke()`, a mock
  backend for `pnpm dev`, Playwright E2E over WebView2 CDP) unless the owner picks otherwise.

## Capture (decided 2026-11-23, after `spikes/etw-spike` and `spikes/npcap-spike`)

The owner: "we should use npcap, with ETW for tying traffic to apps, and as a fallback if the user don't
want npcap", then "Always elevated".

- **Npcap + `pcap` crate is the primary source**: exact bytes and packets, plus hostnames from DNS
  replies and the TLS site name (SNI). The user installs Npcap themselves (free for up to 5 machines;
  bundling or silent install needs a paid OEM license).
- **ETW `Microsoft-Windows-Kernel-Network`** (`7dd42a49-5329-4832-8dfd-43d979153a88`) ties traffic
  to apps: its events carry the process ID plus addresses and ports, so Parcel maps each connection
  to a process ID and tags Npcap packets with it. Without Npcap, ETW is the whole source: bytes only,
  no real packet counts, no hostnames from Chrome's DNS lookups.
- **Always elevated**: ETW needs admin, so capture runs in an elevated Windows service. The Tauri app
  is unelevated UI talking to it.

Spike findings to keep:

- `pcap` links `wpcap.lib` at build time. The spike generated an import library from the installed
  DLL (`dumpbin /exports` → `.def` → `lib.exe`), then used `/DELAYLOAD:wpcap.dll`
  plus `SetDllDirectoryW("C:\Windows\System32\Npcap")` so the exe starts without Npcap.
- Use the manifest Kernel-Network provider, not the legacy kernel logger (`EVENT_TRACE_FLAG_NETWORK_TCPIP`),
  which reports no UDP at all, and most traffic is QUIC over UDP. Send/recv event ids:
  TCP 10/11 (v4) 26/27 (v6), UDP 42/43 (v4) 58/59 (v6). Id 18 duplicates id 11's bytes: skip it.
  `daddr` is always the remote side. Events are send/recv calls, not packets (up to 126 KB each).
- A killed process leaves its ETW session running and the next start fails with `AlreadyExist`:
  stop the session by name before starting.
- Windows' DNS-client ETW events miss Chrome's own DNS lookups (9% of bytes named vs 90% from Npcap's
  DNS replies + SNI). QUIC SNI needs Initial-packet decryption: not built.

## Product calls (decided 2026-11-23, the owner picked from listed options)

- **GeoIP + ASN: DB-IP Lite** (country, city, ASN; MMDB; CC BY 4.0, bundled, monthly updates).
  Credit "IP Geolocation by DB-IP" with a link to db-ip.com. Read it with the `maxminddb` crate.
- **Naming a remote:** SNI / DNS hostname → registered domain (`psl` crate) → curated brand table
  (`googlevideo.com` → YouTube) → ASN org name → raw IP.
- **Logos: fetch the site's own favicon** directly from the site (never through a third-party
  favicon service, which would leak browsing), cached on disk. Parcel's own favicon requests show
  up in its capture.
- **Platforms: Windows first, portable.** Capture and app attribution sit behind an interface so
  macOS / Linux can be added later; only Windows is built.
- **History: SQLite totals at shrinking detail**, per app + remote host, never packets: per second
  in memory for the live graph, per minute 24 h, per hour 30 days, per day indefinitely (periods
  confirmed by the owner 2026-11-23).

## Scope, architecture, shipping (decided 2026-11-23, the owner picked from listed options)

- **v1 is monitor only**: no alerts, no blocking. The owner chose this over the recommended
  "monitor + first-seen alerts".
- **`parcel-service`** is a Windows service (starts at boot, records with the UI closed; admin
  only at install). The UI talks to it over a **named pipe** restricted to logged-in users.
- **Layout like gitgud**: `crates/parcel-core` (naming, aggregation, storage, no Windows capture),
  `crates/parcel-service` (Npcap + ETW capture, the service), `src-tauri` thin client, `src/` Svelte.
- **Npcap first run**: detect it; if missing, a first-run card links to npcap.com and ETW captures
  meanwhile.
- **DB-IP**: the service downloads the new monthly file itself (that traffic shows as Parcel's).
- **Brand table**: hand-curated JSON in the repo (~200 domains to start), ASN owner as fallback.
- **Shipping**: NSIS installer (registers the service) on GitHub Releases with Tauri's updater.
  Code signing starts at the first public release.
- **Tests**: core unit tests, recorded-capture replays through the pipeline, Playwright over
  WebView2 for the GUI (gitgud's approach). Claude's default from the global testing rule, not
  separately asked.

## Architecture (decided 2026-11-23, the owner picked Claude's recommended options)

- **`crates/parcel-ipc`** holds the wire protocol and the named-pipe code, shared by service and UI;
  `src-tauri` depends on it alone, never on `parcel-core`.
- **Length-prefixed JSON** frames on the pipe.
- **The service fetches favicons** (with the DB-IP download) into its ProgramData cache; the UI never
  goes online.
- Capture is one portable `parcel_core::event::Event` stream; only its producers are Windows code.
  With Npcap, Npcap alone counts bytes and ETW only supplies process ids; without it, ETW's
  `Transfer` events are the bytes.

## UI

Design rounds live on `design/lab/index.html` (`design-round` skill), never decided in chat.

- **R1 main window (decided 2026-11-23):** the owner: "A, with B/C tabs". Home is A Timeline: KPI row, a
  mirrored graph with download above the line and upload below, then apps on the left and the selected
  app's hosts on the right. B Atlas (map) and C Flow (app → company ribbons) are tabs. D Manifest
  was not chosen. The tab bar in A's mock (Live · Map · Flow · History) came along without separate
  discussion.

- **R2 (decided 2026-11-23):**
  - Graph, the owner: "mirrored, download and upload by app. Setting to just show 2 colors". Download
    above the line and upload below, both split by app; a setting switches to plain two-colour
    in / out. Default is by app (the owner: "By app"). Drawn in R3.
  - Theme, the owner: "T2": dark + light, following the Windows setting. Every screen is designed and
    checked in both.
  - Parcel touches, the owner: "P2": postage-stamp country codes (straight, not tilted), parcels moving
    on the Map tab only, no tracking numbers.
  - Mini mode, the owner: "M3": a tray flyout with current speed, a 60 s graph and the top apps; closes
    on click-away. No always-on-top mini window.

- **R3 upload scale (decided 2026-11-23):** the owner: "H3". Upload has its own scale, labelled on the
  left (▲ download max, ▼ upload max), in a shorter, paler lower half so download stays the
  headline. The home mock also showed a by-app hover tooltip and a "by app | 2 colours" toggle
  on the graph; those came along without separate discussion.
- **R4 (decided 2026-11-23, the owner picked from listed options):**
  - Light palette **L1 Paper** (warm off-white; values in the lab page's `.light` CSS block).
  - History tab **Y1**: daily bars by app, mirrored like Live (upload paler, shorter); drag a range
    and the tables below follow it. Ranges: 30 days / 12 months / all.
  - First run **O2**: a welcome checklist on first launch (service, Npcap, location data); skipping
    it leaves the R1 state-1 banner until Npcap is installed.
  - Settings **S1**: a gear in the title bar opens a sheet: theme (System / Dark / Light), graph
    (By app / 2 colours), service and Npcap status, DB-IP date + "Update now", history size +
    "Clear…", version and the DB-IP credit.
  - Map and Flow tabs: build from R1's B and C drawings with R2's P2 rules. Tray flyout: build from
    R2's M3 drawing.
- **Parcel's own traffic (decided 2026-11-23):** the owner: "Parcel's own favicon traffic is shown as
  Parcel's traffic". Parcel is listed like any other app.
