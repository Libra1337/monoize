import type { StoreProduct, StoreSettings } from "@/lib/store-api";
import { decimalToMinor, type StoreCurrency } from "@/lib/store-money";

export interface CustomAmountValidation {
  hasCustomAmount: boolean;
  minor: string | null;
  invalid: boolean;
}

export function selectStoreProduct(
  products: StoreProduct[],
  kind: "balance" | "plan",
  selectedId: string | null,
): StoreProduct | null {
  const available = products.filter((product) => product.enabled && product.kind === kind);
  return available.find((product) => product.id === selectedId) ?? available[0] ?? null;
}

export function validateCustomAmount(
  value: string,
  currency: StoreCurrency,
  settings: StoreSettings,
): CustomAmountValidation {
  if (value.trim() === "") {
    return { hasCustomAmount: false, minor: null, invalid: false };
  }

  const minor = decimalToMinor(value);
  if (minor === null) {
    return { hasCustomAmount: true, minor: null, invalid: true };
  }

  const minimum = currency === "CNY"
    ? settings.custom_recharge_cny_min_minor
    : settings.custom_recharge_usd_min_minor;
  const maximum = currency === "CNY"
    ? settings.custom_recharge_cny_max_minor
    : settings.custom_recharge_usd_max_minor;
  const amount = BigInt(minor);
  const invalid = amount < BigInt(minimum) || amount > BigInt(maximum);
  return { hasCustomAmount: true, minor, invalid };
}
