import { describe, expect, it } from "vitest";
import { rate } from "./format";

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
