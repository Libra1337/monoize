import { describe, expect, test } from "bun:test";
import { resolveMarketplacePages } from "../src/lib/marketplace-pages";

interface Page {
  items: Array<{ model: string }>;
}

describe("Public Marketplace cached first page", () => {
  test("keeps the current SWR first page authoritative", () => {
    const current = { items: [{ model: "current-model" }] };
    const stale = { items: [] };
    const next = { items: [{ model: "next-model" }] };
    expect(resolveMarketplacePages<Page>([], current)).toEqual([current]);
    expect(resolveMarketplacePages([stale], current)).toEqual([current]);
    expect(resolveMarketplacePages([stale, next], current)).toEqual([current, next]);
    expect(resolveMarketplacePages([stale], undefined)).toEqual([stale]);
  });
});
