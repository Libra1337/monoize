import { describe, expect, test } from "bun:test";
import { cache } from "swr/_internal";
import { resolveMarketplacePages } from "../src/lib/marketplace-pages";
import { clearCache } from "../src/lib/swr";

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

  test("preserves public SWR data when session cache is cleared", async () => {
    const publicKey = "/api/public/marketplace?limit=50&t=cache-race-test";
    const dashboardKey = "/dashboard/cache-race-test";
    const publicPage = { items: [{ model: "public-model" }] };
    const dashboardData = { private: true };

    cache.set(publicKey, { data: publicPage, _k: publicKey });
    cache.set(dashboardKey, { data: dashboardData, _k: dashboardKey });
    await clearCache();

    expect(cache.get(publicKey)?.data).toEqual(publicPage);
    expect(cache.get(dashboardKey)?.data).toBeUndefined();

    cache.delete(publicKey);
    cache.delete(dashboardKey);
  });
});
