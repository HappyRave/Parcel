const UNITS = ["B", "KB", "MB", "GB", "TB"];

/** Bytes per second as the lab mocks write it: `4.1 MB/s`, `96 KB/s`, `0 B/s`. */
export function rate(bytesPerSec: number): string {
  let value = bytesPerSec;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const shown = unit === 0 || value >= 100 ? Math.round(value).toString() : value.toFixed(1);
  return `${shown} ${UNITS[unit]}/s`;
}
