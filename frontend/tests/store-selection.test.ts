import { describe, expect, test } from "bun:test";
import type { StoreProduct, StoreSettings } from "../src/lib/store-api";
import {
  selectStoreProduct,
  validateCustomAmount,
} from "../src/pages/store/store-selection";

const settings: StoreSettings = {
  custom_recharge_cny_min_minor: "100",
  custom_recharge_cny_max_minor: "10000",
  custom_recharge_usd_min_minor: "25",
  custom_recharge_usd_max_minor: "5000",
};

function product(id: string, kind: "balance" | "plan", enabled = true): StoreProduct {
  return {
    id,
    kind,
    name: id,
    description: "",
    price_currency: "CNY",
    price_minor: "100",
    duration_seconds: null,
    group_ids: [],
    sort_order: 0,
    enabled,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    balance: kind === "balance"
      ? { recharge_minor: "100", bonus_minor: "0", actual_received_minor: "100" }
      : null,
    quotas: [],
  };
}

describe("Store selection", () => {
  test("uses the fixed product amount only when custom input is empty", () => {
    expect(validateCustomAmount("", "CNY", settings)).toEqual({
      hasCustomAmount: false,
      minor: null,
      invalid: false,
    });
  });

  test("rejects malformed and out-of-range custom amounts without a fixed-price fallback", () => {
    expect(validateCustomAmount("abc", "CNY", settings)).toEqual({
      hasCustomAmount: true,
      minor: null,
      invalid: true,
    });
    expect(validateCustomAmount("0.99", "CNY", settings).invalid).toBe(true);
    expect(validateCustomAmount("100.01", "CNY", settings).invalid).toBe(true);
  });

  test("uses bounds from the selected currency", () => {
    expect(validateCustomAmount("1.00", "CNY", settings).invalid).toBe(false);
    expect(validateCustomAmount("0.25", "USD", settings).invalid).toBe(false);
    expect(validateCustomAmount("0.24", "USD", settings).invalid).toBe(true);
  });

  test("keeps the selected balance product while applying a valid custom amount", () => {
    const products = [product("disabled", "balance", false), product("fixed", "balance")];
    const selected = selectStoreProduct(products, "balance", "fixed");
    const custom = validateCustomAmount("12.34", "CNY", settings);

    expect(selected?.id).toBe("fixed");
    expect(custom).toEqual({ hasCustomAmount: true, minor: "1234", invalid: false });
  });
});
