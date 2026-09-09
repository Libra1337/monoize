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
    const nextRevision = { revision: "2", items: ["new-revision"] };
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
    expect(replaceMarketplaceFirstPage(state, "all", nextRevision)).toEqual({
      key: "all",
      pages: [nextRevision],
    });
    expect(appendMarketplacePage(state, "all", nextRevision, 3)).toBe(state);
  });

  // MM-P2a/MM-UA5: `display_rate_nano` arrives in nano-CNY, and `1 C = 1 CNY`, so the client
  // only scales to the display unit and rounds once. No exchange rate is applied here.
  test("renders human per-million prices with exact final rounding", () => {
    expect(formatMarketplaceRate("2505", "token")).toBe("C2.51 / 1M tokens");
    expect(formatMarketplaceRate("2504.999999999", "token")).toBe("C2.50 / 1M tokens");
    expect(formatMarketplaceRate("1250000000.5", "call")).toBe("C1.25 / call");
    // A range carries one leading symbol; the upper bound omits it.
    expect(formatMarketplaceRateRange({ min: "1000", max: "2500", unit: "token" })).toBe(
      "C1.00–2.50 / 1M tokens",
    );
  });

  test("keeps the route inside DashboardLayout and uses public allow-listed data", () => {
    // MM-UA1 requires the route to stay inside the dashboard shell. The shell is now
    // wrapped by the SC-UI-1 guard that redirects a sales agent to their own surface, so the
    // assertion checks the nesting rather than one exact element string.
    expect(appSource).toContain('<Route path="/dashboard" element={<DashboardGuard><DashboardLayout /></DashboardGuard>}>');
    expect(appSource).toContain('<Route path="marketplace" element={<ModelMarketplacePage />} />');
    expect(appSource).toContain('<Route path="marketplace" element={<ModelMarketplacePage />} />');
    expect(marketplaceSource).toContain("/api/public/marketplace?");
    expect(marketplaceSource).toContain("/api/public/marketplace/offers?");
    // MM-UA5: the page must not reach for the exchange rate or a display currency.
    expect(marketplaceSource).not.toContain("useStoreExchangeRate");
    expect(marketplaceSource).not.toContain("useStoreCurrency");
    expect(marketplaceSource).toContain("keepPreviousData: true");
    expect(marketplaceSource).toContain("<Dialog");
    expect(marketplaceSource).toContain("public_group_name");
    expect(marketplaceSource).toContain("capabilities");
    expect(marketplaceSource).toContain("selected?.capabilities.map");
    expect(marketplaceSource).toContain("list.mutate()");
    expect(marketplaceSource).toContain("offers.mutate()");
    expect(marketplaceSource.match(/rememberGroups\(page\);/g)?.length).toBe(2);
    expect(marketplaceSource).toContain("offerLoadCursor");
    expect(marketplaceSource).toContain("selected.revision");
    expect(marketplaceSource).toContain("JSON.stringify([selected.revision");
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
      marketplaceSource.indexOf("function ModelRow"),
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
