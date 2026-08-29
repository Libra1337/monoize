import { describe, expect, test } from "bun:test";
import {
  addMinor,
  convertMinor,
  formatMinor,
  formatPerMillionTokenRate,
  formatPlanQuota,
  minorToDecimal,
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

  test("adds separately converted recharge and bonus amounts without a second rounding step", () => {
    const recharge = convertMinor("1", "CNY", "USD", "2");
    const bonus = convertMinor("1", "CNY", "USD", "2");

    expect(addMinor(recharge, bonus)).toBe("2");
    expect(convertMinor("2", "CNY", "USD", "2")).toBe("1");
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

  test("converts minor units to an exact editable decimal", () => {
    expect(minorToDecimal("0")).toBe("0.00");
    expect(minorToDecimal("5")).toBe("0.05");
    expect(minorToDecimal("1250")).toBe("12.50");
  });

  test("formats per-token nano USD rates as human prices per one million tokens", () => {
    expect(formatPerMillionTokenRate("1500", "USD", "7")).toBe("$1.50 / 1M tokens");
    expect(formatPerMillionTokenRate("1500", "CNY", "7")).toBe("¥10.50 / 1M tokens");
    expect(formatPerMillionTokenRate("0.5", "USD", "7")).toBe("$0.00 / 1M tokens");
  });
});
