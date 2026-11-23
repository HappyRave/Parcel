import { describe, expect, it } from "vitest";
import { bytes, count, parts, rate } from "./format";

describe("parts", () => {
  it("splits the number from its unit for KPIs", () => {
    expect(parts(4_100_000)).toEqual(["4.1", "MB"]);
    expect(parts(96)).toEqual(["96", "B"]);
  });
});

describe("count", () => {
  it("groups thousands", () => {
    expect(count(30072)).toBe("30,072");
    expect(count(7)).toBe("7");
  });
});

describe("bytes", () => {
  it("uses the same steps without a per-second suffix", () => {
    expect(bytes(0)).toBe("0 B");
    expect(bytes(1_234_567_890)).toBe("1.2 GB");
    expect(bytes(250_400)).toBe("250 KB");
  });
});

describe("rate", () => {
  it("keeps bytes whole", () => {
    expect(rate(0)).toBe("0 B/s");
    expect(rate(999)).toBe("999 B/s");
  });

  it("shows one decimal below 100", () => {
    expect(rate(4_100_000)).toBe("4.1 MB/s");
    expect(rate(1000)).toBe("1.0 KB/s");
  });

  it("drops the decimal from 100 up", () => {
    expect(rate(96_400)).toBe("96.4 KB/s");
    expect(rate(250_400)).toBe("250 KB/s");
  });

  it("stops at TB", () => {
    expect(rate(5e15)).toBe("5000 TB/s");
  });
});
