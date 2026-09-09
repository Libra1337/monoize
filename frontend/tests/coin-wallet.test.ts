import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  formatCoinFromNanoUsd,
  formatCoinPerUnitCny,
  formatCoinFromMinor,
  formatCoinFromMinorForCurrency,
  formatCoinFromNanoUsdForCurrency,
} from "../src/lib/store-money";

describe("Coin display", () => {
  const walletSource = readFileSync(new URL("../src/pages/wallet.tsx", import.meta.url), "utf8");
  const apiSource = readFileSync(new URL("../src/lib/api.ts", import.meta.url), "utf8");

  test("converts wallet nano-USD balances through the CNY/USD snapshot", () => {
    expect(formatCoinFromNanoUsd("1000000000", "7.2")).toBe("C7.20");
  });

  // CN-4: a published rate is already CNY, so `1 C = 1 CNY` needs no exchange rate.
  test("renders a server-normalized CNY rate as Coin without conversion", () => {
    expect(formatCoinPerUnitCny("1000", "token")).toBe("C1.00 / 1M tokens");
    expect(formatCoinPerUnitCny("10000000", "token")).toBe("C10000.00 / 1M tokens");
    expect(formatCoinPerUnitCny("1250000000.5", "call")).toBe("C1.25 / call");
  });

  test("uses CNY as the Coin basis for CNY recharge products", () => {
    expect(formatCoinFromMinor("1234", "CNY", "7.2")).toBe("C12.34");
  });

  test("converts USD recharge products to equivalent Coin at the quote rate", () => {
    expect(formatCoinFromMinor("1000", "USD", "7.2")).toBe("C72.00");
  });

  test("changes Coin display values when the selected currency changes", () => {
    expect(formatCoinFromMinorForCurrency("720", "CNY", "CNY", "7.2")).toBe("C7.20");
    expect(formatCoinFromMinorForCurrency("720", "CNY", "USD", "7.2")).toBe("C1.00");
    expect(formatCoinFromNanoUsdForCurrency("1000000000", "CNY", "7.2")).toBe("C7.20");
    expect(formatCoinFromNanoUsdForCurrency("1000000000", "USD", "7.2")).toBe("C1.00");
  });

  test("wallet exposes a user-scoped ledger and Coin mark", () => {
    expect(walletSource).toContain("const ledger = useSWR");
    expect(walletSource).toContain("api.listWalletLedger(ledgerLimit)");
    expect(apiSource).toContain('this.request(`/wallet/ledger?limit=');
    expect(apiSource).not.toContain('this.request(`/dashboard/wallet/ledger?limit=');
    expect(walletSource).toContain("CoinAmount");
    expect(walletSource).toContain("ledgerLimit");
    expect(walletSource).toContain("api.listWalletLedger(ledgerLimit)");
  });
});
