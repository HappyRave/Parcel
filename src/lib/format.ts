const UNITS = ["B", "KB", "MB", "GB", "TB"];

/** An amount of data as the lab mocks write it: `4.1 MB`, `96 KB`, `0 B`. */
export function bytes(n: number): string {
  let value = n;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const shown = unit === 0 || value >= 100 ? Math.round(value).toString() : value.toFixed(1);
  return `${shown} ${UNITS[unit]}`;
}

/** Bytes per second: `4.1 MB/s`, `96 KB/s`, `0 B/s`. */
export function rate(bytesPerSec: number): string {
  return `${bytes(bytesPerSec)}/s`;
}
