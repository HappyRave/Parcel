const UNITS = ["B", "KB", "MB", "GB", "TB"];

/** An amount split for a KPI's big number and small unit: `[4.1, "MB"]` shown as `4.1 MB`. */
export function parts(n: number): [string, string] {
  let value = n;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const shown = unit === 0 || value >= 100 ? Math.round(value).toString() : value.toFixed(1);
  return [shown, UNITS[unit]];
}

/** An amount of data as the lab mocks write it: `4.1 MB`, `96 KB`, `0 B`. */
export function bytes(n: number): string {
  return parts(n).join(" ");
}

/** Bytes per second: `4.1 MB/s`, `96 KB/s`, `0 B/s`. */
export function rate(bytesPerSec: number): string {
  return `${bytes(bytesPerSec)}/s`;
}

/** A count with thousands separators: `30,072`. */
export function count(n: number): string {
  return Math.round(n).toLocaleString("en-US");
}

/** Local wall-clock time of `ms`: `14:32:08`. */
export function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString("en-GB", { hour12: false });
}
