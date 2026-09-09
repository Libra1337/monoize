import { describe, expect, test } from "bun:test";
import {
  addMinor,
  balanceOrderReceivedMinor,
  convertMinor,
  formatMinor,
  formatNanoUsd,
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

  test("formats signed account balances without crashing the dashboard", () => {
    expect(formatNanoUsd("-1000000000", "USD", "1")).toBe("-$1.00");
    expect(formatNanoUsd("-1000000000", "CNY", "7")).toBe("-¥7.00");
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

  // SB-UI-10D, reproduced from the production order that displayed "50" for a 1 CNY custom
  // recharge. The carrier product's name and price describe the tier whose row the custom
  // amount borrowed, so only the balance block states what the buyer received.
  test("reads what a balance order delivered rather than its carrier product", () => {
    const customRecharge = {
      kind: "balance",
      name: "50",
      price_minor: "5000",
      balance: {
        recharge_minor: "100",
        bonus_minor: "0",
        actual_received_minor: "100",
      },
    };
    expect(balanceOrderReceivedMinor(customRecharge)).toBe("100");
    expect(formatMinor(balanceOrderReceivedMinor(customRecharge)!, "CNY")).toBe("¥1.00");

    // A fixed tier keeps working, and a bonus counts toward what was received.
    const fixedTier = {
      kind: "balance",
      name: "100",
      price_minor: "10000",
      balance: {
        recharge_minor: "10000",
        bonus_minor: "2000",
        actual_received_minor: "12000",
      },
    };
    expect(balanceOrderReceivedMinor(fixedTier)).toBe("12000");

    // A plan order has no balance block and keeps its own name as the title.
    expect(
      balanceOrderReceivedMinor({ kind: "plan", name: "Pro monthly", balance: null }),
    ).toBeNull();
  });
});
