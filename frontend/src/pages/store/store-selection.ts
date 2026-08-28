import type {
  PaymentAdapterKind,
  StoreAmountLimit,
  StoreCheckoutActionKind,
  StorePaymentChannel,
  StoreProduct,
  StoreSettings,
} from "@/lib/store-api";
import { decimalToMinor, type StoreCurrency } from "@/lib/store-money";

export interface CustomAmountValidation {
  hasCustomAmount: boolean;
  minor: string | null;
  invalid: boolean;
}

export type CheckoutViewport = "mobile" | "desktop";

const CURRENCIES = new Set<StoreCurrency>(["CNY", "USD"]);
const ACTIONS = new Set<StoreCheckoutActionKind>(["redirect", "qr", "form"]);
const CANONICAL_POSITIVE_INTEGER = /^[1-9][0-9]*$/;

function requiredAction(
  adapterKind: PaymentAdapterKind,
  viewport: CheckoutViewport,
): StoreCheckoutActionKind | null {
  if (adapterKind === "wechat") return viewport === "mobile" ? "redirect" : "qr";
  if (adapterKind === "alipay") return "form";
  if (adapterKind === "stripe") return "redirect";
  return null;
}

function isExactAmountLimit(value: unknown): value is StoreAmountLimit {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  const keys = Object.keys(value).sort();
  if (keys.length !== 2 || keys[0] !== "max_minor" || keys[1] !== "min_minor") return false;
  const limit = value as Record<string, unknown>;
  if (typeof limit.min_minor !== "string" || typeof limit.max_minor !== "string") return false;
  return (
    CANONICAL_POSITIVE_INTEGER.test(limit.min_minor)
    && CANONICAL_POSITIVE_INTEGER.test(limit.max_minor)
    && BigInt(limit.min_minor) <= BigInt(limit.max_minor)
  );
}

function hasValidMetadata(channel: StorePaymentChannel): boolean {
  if (
    channel.enabled !== true
    || channel.effective_available !== true
    || !Array.isArray(channel.unavailable_reasons)
    || channel.unavailable_reasons.length !== 0
    || channel.adapter_kind === "http"
  ) {
    return false;
  }
  if (!["alipay", "wechat", "stripe"].includes(channel.adapter_kind)) return false;
  if (!Array.isArray(channel.supported_currencies) || channel.supported_currencies.length === 0) {
    return false;
  }
  const currencyNames = channel.supported_currencies;
  const currencySet = new Set(currencyNames);
  if (currencySet.size !== currencyNames.length || currencyNames.some((item) => !CURRENCIES.has(item))) {
    return false;
  }
  if (
    (channel.adapter_kind === "alipay" || channel.adapter_kind === "wechat")
    && (currencyNames.length !== 1 || currencyNames[0] !== "CNY")
  ) {
    return false;
  }

  if (!channel.amount_limits || typeof channel.amount_limits !== "object" || Array.isArray(channel.amount_limits)) {
    return false;
  }
  const limitKeys = Object.keys(channel.amount_limits);
  if (
    limitKeys.length !== currencySet.size
    || limitKeys.some((key) => !currencySet.has(key as StoreCurrency))
  ) {
    return false;
  }
  for (const currency of currencyNames) {
    const limit = channel.amount_limits[currency];
    if (!isExactAmountLimit(limit)) return false;
  }

  if (!Array.isArray(channel.checkout_action_kinds) || channel.checkout_action_kinds.length === 0) {
    return false;
  }
  const actions = channel.checkout_action_kinds;
  const actionSet = new Set(actions);
  if (actionSet.size !== actions.length || actions.some((action) => !ACTIONS.has(action))) return false;
  if (channel.adapter_kind === "alipay") return actions.length === 1 && actions[0] === "form";
  if (channel.adapter_kind === "stripe") return actions.length === 1 && actions[0] === "redirect";
  return actions.every((action) => action === "qr" || action === "redirect");
}

export function filterCompatiblePaymentChannels(
  channels: StorePaymentChannel[],
  currency: StoreCurrency,
  paymentMinor: string | null,
  viewport: CheckoutViewport,
): StorePaymentChannel[] {
  if (!paymentMinor || !CANONICAL_POSITIVE_INTEGER.test(paymentMinor)) return [];
  const amount = BigInt(paymentMinor);
  return channels.filter((channel) => {
    if (!hasValidMetadata(channel) || !channel.supported_currencies.includes(currency)) return false;
    const limit = channel.amount_limits[currency];
    const action = requiredAction(channel.adapter_kind, viewport);
    return Boolean(
      limit
      && action
      && amount >= BigInt(limit.min_minor)
      && amount <= BigInt(limit.max_minor)
      && channel.checkout_action_kinds.includes(action),
    );
  });
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
