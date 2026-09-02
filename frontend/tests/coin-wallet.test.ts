import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  formatCoinFromNanoUsd,
  formatCoinPerMillionUsd,
  formatCoinFromMinor,
} from "../src/lib/store-money";

describe("Coin display", () => {
  const walletSource = readFileSync(new URL("../src/pages/wallet.tsx", import.meta.url), "utf8");
  const apiSource = readFileSync(new URL("../src/lib/api.ts", import.meta.url), "utf8");

  test("converts wallet nano-USD balances through the CNY/USD snapshot", () => {
    expect(formatCoinFromNanoUsd("1000000000", "7.2")).toBe("C7.20");
  });

  test("keeps USD-basis model prices numerically stable in Coin", () => {
    expect(formatCoinPerMillionUsd("1000")).toBe("C1.00 / 1M tokens");
    expect(formatCoinPerMillionUsd("10000000")).toBe("C10000.00 / 1M tokens");
  });

  test("uses CNY as the Coin basis for CNY recharge products", () => {
    expect(formatCoinFromMinor("1234", "CNY", "7.2")).toBe("C12.34");
  });

  test("converts USD recharge products to equivalent Coin at the quote rate", () => {
    expect(formatCoinFromMinor("1000", "USD", "7.2")).toBe("C72.00");
  });

  test("wallet exposes a user-scoped ledger and Coin mark", () => {
    expect(walletSource).toContain("const ledger = useSWR");
    expect(walletSource).toContain("useSWR(\"/api/dashboard/wallet/ledger\", () => api.listWalletLedger())");
    expect(apiSource).toContain('this.request(`/wallet/ledger?limit=');
    expect(apiSource).not.toContain('this.request(`/dashboard/wallet/ledger?limit=');
    expect(walletSource).toContain("CoinAmount");
  });
});
