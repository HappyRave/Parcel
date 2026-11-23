import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { Commands, ServiceState, Tick } from "$lib/api";

/** Dev-only fake backend: `pnpm dev`, then open `/?mock`. Ticks a wobbly download every second. */
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
    const tick: Tick = {
      at: Math.floor(Date.now() / 1000) * 1000,
      down: Math.round(2_000_000 + 1_500_000 * Math.sin(t / 5)),
      up: Math.round(80_000 + 40_000 * Math.cos(t / 3)),
    };
    void emit("tick", tick);
  }, 1000);
}
