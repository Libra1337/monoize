import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const componentSource = readFileSync(
  new URL("../src/components/api-key-analytics-dialog.tsx", import.meta.url),
  "utf8",
);
const pageSource = readFileSync(
  new URL("../src/pages/api-keys.tsx", import.meta.url),
  "utf8",
);
const swrSource = readFileSync(
  new URL("../src/lib/swr.ts", import.meta.url),
  "utf8",
);

describe("API Key analytics dialog", () => {
  test("opens from the compact key list without navigation", () => {
    expect(pageSource).toContain("ApiKeyAnalyticsDialog");
    expect(pageSource).toContain("setAnalyticsKey(key)");
    expect(pageSource).not.toContain("/tokens/${key.id}/analytics");
  });

  test("keeps stale values while changing all four ranges", () => {
    expect(swrSource).toContain("useApiKeyAnalytics");
    expect(swrSource).toContain("keepPreviousData: true");
    for (const range of ["24h", "7d", "30d", "all"]) {
      expect(componentSource).toContain(`value="${range}"`);
    }
    expect(componentSource).toContain("isValidating");
    expect(componentSource).toContain("<Skeleton");
  });

  test("shows balance, exact token totals, trend, and model rows", () => {
    expect(componentSource).toContain("independent_balance_nano");
    expect(componentSource).toContain("AnimatedTokenValue");
    expect(componentSource).toContain("UsageTrendChart");
    expect(componentSource).toContain("data.models.map");
    expect(componentSource).toContain("formatCoinFromNanoUsdForCurrency");
  });

  test("defines analytics copy in every locale", () => {
    for (const locale of ["en", "zh", "zh-TW", "ja"]) {
      const catalog = JSON.parse(
        readFileSync(new URL(`../src/locales/${locale}.json`, import.meta.url), "utf8"),
      );
      for (const key of [
        "analyticsTitle",
        "analyticsDescription",
        "analyticsRequests",
        "analyticsCost",
        "analyticsModels",
        "analyticsWalletBalance",
        "analyticsIndependentBalance",
      ]) {
        expect(typeof catalog.apiKeys?.[key], `${locale}: ${key}`).toBe("string");
      }
    }
  });
});
