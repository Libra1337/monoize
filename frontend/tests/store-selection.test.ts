import { describe, expect, test } from "bun:test";
import type {
  StoreAmountLimit,
  StorePaymentChannel,
  StoreProduct,
  StoreSettings,
} from "../src/lib/store-api";
import {
  filterCompatiblePaymentChannels,
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

function channel(
  id: string,
  adapterKind: StorePaymentChannel["adapter_kind"],
  overrides: Partial<StorePaymentChannel> = {},
): StorePaymentChannel {
  return {
    id,
    adapter_kind: adapterKind,
    name: id,
    icon_kind: "builtin",
    icon_value: null,
    sort_order: 0,
    enabled: true,
    revision: 1,
    effective_available: true,
    unavailable_reasons: [],
    supported_currencies: adapterKind === "stripe" ? ["CNY", "USD"] : ["CNY"],
    amount_limits: adapterKind === "stripe"
      ? {
          CNY: { min_minor: "1", max_minor: "999999999999999999999999" },
          USD: { min_minor: "1", max_minor: "999999999999999999999999" },
        }
      : { CNY: { min_minor: "1", max_minor: "999999999999999999999999" } },
    checkout_action_kinds: adapterKind === "wechat"
      ? ["qr", "redirect"]
      : adapterKind === "alipay"
        ? ["form"]
        : ["redirect"],
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
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

  test("filters Channels by currency and exact BigInt amount bounds", () => {
    const channels = [
      channel("alipay", "alipay"),
      channel("stripe", "stripe", {
        amount_limits: {
          CNY: { min_minor: "9007199254740993", max_minor: "9007199254740995" },
          USD: { min_minor: "9007199254740993", max_minor: "9007199254740995" },
        },
      }),
    ];

    expect(filterCompatiblePaymentChannels(
      channels,
      "USD",
      "9007199254740993",
      "desktop",
    ).map((item) => item.id)).toEqual(["stripe"]);
    expect(filterCompatiblePaymentChannels(
      channels,
      "USD",
      "9007199254740995",
      "desktop",
    ).map((item) => item.id)).toEqual(["stripe"]);
    expect(filterCompatiblePaymentChannels(
      channels,
      "USD",
      "9007199254740996",
      "desktop",
    )).toEqual([]);
  });

  test("uses the required WeChat action for the current viewport", () => {
    const qrOnly = channel("wechat-qr", "wechat", { checkout_action_kinds: ["qr"] });
    const redirectOnly = channel("wechat-h5", "wechat", { checkout_action_kinds: ["redirect"] });

    expect(filterCompatiblePaymentChannels(
      [qrOnly, redirectOnly],
      "CNY",
      "100",
      "desktop",
    ).map((item) => item.id)).toEqual(["wechat-qr"]);
    expect(filterCompatiblePaymentChannels(
      [qrOnly, redirectOnly],
      "CNY",
      "100",
      "mobile",
    ).map((item) => item.id)).toEqual(["wechat-h5"]);
  });

  test("fails closed for HTTP, unavailable, unknown, and malformed metadata", () => {
    const candidates = [
      channel("http", "http"),
      channel("unavailable", "stripe", { effective_available: false }),
      channel("bad-amount", "stripe", {
        amount_limits: {
          CNY: { min_minor: "01", max_minor: "100" },
          USD: { min_minor: "1", max_minor: "100" },
        },
      }),
      channel("missing-limit", "stripe", { amount_limits: { CNY: { min_minor: "1", max_minor: "100" } } }),
      channel("unknown-action", "stripe", { checkout_action_kinds: ["popup" as "redirect"] }),
      channel("duplicate-currency", "stripe", { supported_currencies: ["USD", "USD"] }),
      channel("unknown-adapter", "stripe", { adapter_kind: "cash" as "stripe" }),
      channel("bad-reasons", "stripe", { unavailable_reasons: undefined as unknown as string[] }),
      channel("numeric-limit", "stripe", {
        amount_limits: {
          CNY: { min_minor: "1", max_minor: "100" },
          USD: { min_minor: 1, max_minor: 100 } as unknown as StoreAmountLimit,
        },
      }),
      channel("null-limit", "stripe", {
        amount_limits: {
          CNY: { min_minor: "1", max_minor: "100" },
          USD: null as unknown as StoreAmountLimit,
        },
      }),
      channel("extra-limit-field", "stripe", {
        amount_limits: {
          CNY: { min_minor: "1", max_minor: "100" },
          USD: { min_minor: "1", max_minor: "100", mode: "inclusive" } as StoreAmountLimit,
        },
      }),
    ];

    expect(filterCompatiblePaymentChannels(candidates, "USD", "50", "desktop")).toEqual([]);
    expect(filterCompatiblePaymentChannels(
      [channel("valid", "stripe")],
      "USD",
      "050",
      "desktop",
    )).toEqual([]);
  });
});
