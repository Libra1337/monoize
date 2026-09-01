import { describe, expect, test } from "bun:test";
import {
  formatCoinFromNanoUsd,
  formatCoinPerMillionUsd,
  formatCoinFromMinor,
} from "../src/lib/store-money";

describe("Coin display", () => {
  test("converts wallet nano-USD balances through the CNY/USD snapshot", () => {
    expect(formatCoinFromNanoUsd("1000000000", "7.2")).toBe("C7.20");
  });

  test("keeps USD-basis model prices numerically stable in Coin", () => {
    expect(formatCoinPerMillionUsd("1000")).toBe("C1.00 / 1M tokens");
    expect(formatCoinPerMillionUsd("10000000")).toBe("C10,000.00 / 1M tokens");
  });

  test("uses CNY as the Coin basis for CNY recharge products", () => {
    expect(formatCoinFromMinor("1234", "CNY", "7.2")).toBe("C12.34");
  });

  test("converts USD recharge products to equivalent Coin at the quote rate", () => {
    expect(formatCoinFromMinor("1000", "USD", "7.2")).toBe("C72.00");
  });
});
