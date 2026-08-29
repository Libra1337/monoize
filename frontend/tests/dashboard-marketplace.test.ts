import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  formatMarketplaceRate,
  formatMarketplaceRateRange,
} from "../src/lib/marketplace-pricing";
import {
  appendMarketplacePage,
  replaceMarketplaceFirstPage,
} from "../src/lib/marketplace-pages";

const marketplaceSource = readFileSync(
  new URL("../src/pages/model-marketplace.tsx", import.meta.url),
  "utf8",
);
const appSource = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");

describe("authenticated Model Marketplace", () => {
  test("retains pages on revalidation and rejects stale pagination", () => {
    const first = { revision: "1", items: ["first"] };
    const second = { revision: "1", items: ["second"] };
    const refreshed = { revision: "1", items: ["refreshed"] };
    const state = { key: "all", pages: [first, second] };

    expect(replaceMarketplaceFirstPage(state, "all", refreshed)).toEqual({
      key: "all",
      pages: [refreshed, second],
    });
    expect(replaceMarketplaceFirstPage(state, "group-a", refreshed)).toEqual({
      key: "group-a",
      pages: [refreshed],
    });
    expect(appendMarketplacePage(state, "stale-key", refreshed, 3)).toBe(state);
    expect(appendMarketplacePage(state, "all", refreshed, 2)).toEqual({
      key: "all",
      pages: [second, refreshed],
    });
  });

  test("renders human per-million prices with exact final rounding", () => {
    expect(formatMarketplaceRate("2505", "token", "USD", "7.200000")).toBe(
      "$2.51 / 1M tokens",
    );
    expect(formatMarketplaceRate("2504.999999999", "token", "USD", "7.200000")).toBe(
      "$2.50 / 1M tokens",
    );
    expect(formatMarketplaceRate("2500.000000001", "token", "CNY", "7.200000")).toBe(
      "¥18.00 / 1M tokens",
    );
    expect(formatMarketplaceRate("1250000000.5", "call", "USD", "7.200000")).toBe(
      "$1.25 / call",
    );
    expect(formatMarketplaceRateRange(
      { min: "1000", max: "2500", unit: "token" },
      "USD",
      "7.200000",
    )).toBe("$1.00–$2.50 / 1M tokens");
  });

  test("keeps the route inside DashboardLayout and uses public allow-listed data", () => {
    expect(appSource).toContain('<Route path="/dashboard" element={<DashboardLayout />}>');
    expect(appSource).toContain('<Route path="marketplace" element={<ModelMarketplacePage />} />');
    expect(marketplaceSource).toContain("/api/public/marketplace?");
    expect(marketplaceSource).toContain("/api/public/marketplace/offers?");
    expect(marketplaceSource).toContain("storeApi.getExchangeRate");
    expect(marketplaceSource).toContain("useStoreCurrency");
    expect(marketplaceSource).toContain("keepPreviousData: true");
    expect(marketplaceSource).toContain("<Dialog");
    expect(marketplaceSource).toContain("public_group_name");
    expect(marketplaceSource).toContain("capabilities");
    expect(marketplaceSource).toContain("selected?.capabilities.map");
    expect(marketplaceSource).toContain("exchangeRate.mutate()");
    expect(marketplaceSource.match(/rememberGroups\(page\);/g)?.length).toBe(2);
    expect(marketplaceSource).toContain("offerLoadCursor");
    expect(marketplaceSource).toContain("selected.revision");
    expect(marketplaceSource).toContain("appendMarketplacePage");
  });

  test("uses naturally expanding rows and does not expose private catalog values", () => {
    expect(marketplaceSource).not.toContain("TableVirtuoso");
    expect(marketplaceSource).not.toContain("useMarketplaceModels");
    expect(marketplaceSource).not.toContain("localStorage");
    expect(marketplaceSource).not.toContain("input_cost_per_token_nano");
    expect(marketplaceSource).not.toContain("output_cost_per_token_nano");
    expect(marketplaceSource).not.toContain("models_dev_provider");
    expect(marketplaceSource).not.toContain("nano-USD /");
    const skeletonSource = marketplaceSource.slice(
      marketplaceSource.indexOf("function MarketplaceSkeleton"),
      marketplaceSource.indexOf("function CurrencyControl"),
    );
    expect(skeletonSource).not.toContain("lg:grid-cols");
  });

  test("defines the complete Console Marketplace copy in every locale", () => {
    for (const locale of ["en", "zh", "zh-TW", "ja"]) {
      const catalog = JSON.parse(
        readFileSync(new URL(`../src/locales/${locale}.json`, import.meta.url), "utf8"),
      );
      const marketplace = catalog.modelMarketplace;
      expect(marketplace).toBeDefined();
      for (const key of [
        "title",
        "description",
        "searchPlaceholder",
        "groupFilter",
        "allGroups",
        "capabilityFilter",
        "allCapabilities",
        "currency",
        "inputPrice",
        "outputPrice",
        "offerCount",
        "detailsDescription",
        "provider",
        "channel",
        "apiType",
        "capabilities",
        "loadError",
        "offersError",
        "retry",
        "noModels",
        "noModelsDesc",
        "unavailable",
      ]) {
        expect(typeof marketplace[key], `${locale}: ${key}`).toBe("string");
      }
    }
  });
});
