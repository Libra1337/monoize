import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolveMarketplacePages } from "../src/lib/marketplace-pages";

const marketplaceSource = readFileSync(
  new URL("../src/pages/public-marketplace.tsx", import.meta.url),
  "utf8",
);

interface Page {
  items: Array<{ model: string }>;
}

describe("Public Marketplace cached first page", () => {
  test("uses SWR data until retained pages are populated", () => {
    const cached = { items: [{ model: "cached-model" }] };
    const retained = { items: [{ model: "retained-model" }] };
    expect(resolveMarketplacePages<Page>([], cached)).toEqual([cached]);
    expect(resolveMarketplacePages([retained], cached)).toEqual([retained]);
    expect(resolveMarketplacePages<Page>([], undefined)).toEqual([]);
  });

  test("bypasses stale browser HTTP cache entries", () => {
    expect(marketplaceSource).toContain('cache: "no-store"');
  });
});
