import { describe, expect, test } from "bun:test";
import { resolveMarketplacePages } from "../src/lib/marketplace-pages";

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
});
