# Parcel

A lightweight network monitor for Windows. Parcel shows your PC's live traffic and where every packet goes:
which app sent it, which company or service received it, in which country, how many packets and how much data.

> **Status: scaffold.** The capture approach is proven by two spikes, and the UI has gone through four
> design rounds. The service, app and pipe between them run end to end, but nothing is captured yet.

## What it will do

- **Live graph:** download above the line and upload below, both split by app, over 5 minutes to 30 days.
- **Who's talking:** every app, and every host it talks to, named by service (YouTube, GitHub, …) with the site's
  favicon, location and network owner.
- **Map and Flow:** a world map of where your data goes, and ribbons from apps to the companies they talk to.
- **History:** per-minute detail for 24 hours, per-hour for 30 days, per-day indefinitely, kept in a local SQLite file.
- **Tray flyout:** current speed, the last minute and the top apps, one click away.
- **Dark and light** themes, following Windows.

v1 is a monitor only: no alerts and no blocking.

## How it works

- **[Npcap](https://npcap.com)** captures packets: exact byte and packet counts, plus hostnames from DNS replies and
  the TLS site name.
- **ETW** (Windows' `Microsoft-Windows-Kernel-Network` events) tells Parcel which process owns each connection.
  Without Npcap, ETW is the whole source: bytes per app still work, but packet counts and most hostnames don't.
- Capture runs in an elevated **Windows service** that records even with the window closed. The app is a normal,
  unelevated UI that talks to it over a named pipe.
- Locations and network owners come from **[DB-IP Lite](https://db-ip.com)**, updated monthly.

Npcap isn't bundled: its license asks users to install it themselves (free on up to 5 machines). Parcel detects it
on first run and links to npcap.com.

## Stack

Rust, [Tauri 2](https://tauri.app) and SvelteKit (Svelte 5). All logic lives in Rust; the Svelte side only renders.

Layout:

```
crates/parcel-core      naming, aggregation, storage (portable, no capture code)
crates/parcel-ipc       the service ↔ app protocol over a named pipe
crates/parcel-service   Npcap + ETW capture, the Windows service
src-tauri               thin Tauri client
src                     SvelteKit UI
```

Running it (the scaffold: the service serves empty ticks until capture is ported):

```powershell
pnpm install
cargo run -p parcel-service -- console
pnpm tauri dev
```

## In this repo today

- [`design/lab/index.html`](design/lab/index.html): the design lab. Open it in a browser to see every round, the options,
  and what was decided.
- [`spikes/etw-spike`](spikes/etw-spike) and [`spikes/npcap-spike`](spikes/npcap-spike): throwaway programs that compared
  the two capture methods on the same traffic. Both print a 60-second report against the network adapter's own counters.

Running the spikes (Windows, Rust stable, run from an **admin** terminal for ETW):

```powershell
cargo run --release --manifest-path spikes/etw-spike/Cargo.toml -- 60
cargo run --release --manifest-path spikes/npcap-spike/Cargo.toml -- 60
```

The Npcap spike needs Npcap installed and links against `wpcap.lib`. Instead of downloading the Npcap SDK, generate
the import library from the installed DLL with the Visual Studio tools, in `spikes/npcap-spike/lib`:

```powershell
lib /MACHINE:X64 /DEF:wpcap.def /OUT:wpcap.lib
```

## Credits

Parcel will ship IP geolocation by [DB-IP](https://db-ip.com), licensed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).

## License

[MIT](LICENSE)
