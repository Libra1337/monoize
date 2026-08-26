import { describe, expect, test } from "bun:test";
import {
  convertMinor,
  formatMinor,
  formatPlanQuota,
} from "../src/lib/store-money";

describe("Store money helpers", () => {
  test("formats CNY and USD minor-unit strings without floating-point arithmetic", () => {
    expect(formatMinor("1250", "CNY")).toBe("¥12.50");
    expect(formatMinor("1250", "USD")).toBe("$12.50");
    expect(formatMinor("5", "CNY")).toBe("¥0.05");
  });

  test("converts CNY and USD minor units at the exact decimal rate", () => {
    expect(convertMinor("5900", "CNY", "USD", "6.7370")).toBe("876");
    expect(convertMinor("876", "USD", "CNY", "6.7370")).toBe("5902");
  });

  test("rounds plan quota from its CNY base to whole display units", () => {
    expect(formatPlanQuota("2000", "CNY", "6.7370")).toBe("¥20");
    expect(formatPlanQuota("6800", "CNY", "6.7370")).toBe("¥68");
    expect(formatPlanQuota("2000", "USD", "6.7370")).toBe("$3");
    expect(formatPlanQuota("6800", "USD", "6.7370")).toBe("$10");
  });

  test("rejects non-canonical amounts and invalid exchange rates", () => {
    expect(() => formatMinor("01", "CNY")).toThrow("canonical");
    expect(() => convertMinor("100", "CNY", "USD", "0")).toThrow("rate");
  });
});
