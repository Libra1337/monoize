import type { StoreCurrency } from "@/lib/store-money";

const STORE_API_BASE = "/api/dashboard/store";

export type ProductKind = "balance" | "plan";
export type PaymentChannelKind = "alipay" | "wechat" | "custom";
export type PaymentChannelMode = "redirect" | "qr" | "manual";
export type PaymentChannelIconKind = "builtin" | "url" | "upload";
export type PlanWindowKind = "5h" | "12h" | "day" | "week" | "month" | "custom";
export type StoreOrderStatus = "pending" | "completed" | "cancelled";
export type RedemptionCodeStatus = "unused" | "used";

export interface BalanceProductInput {
  recharge_minor: string;
  bonus_minor: string;
}

export interface StoreBalanceProduct extends BalanceProductInput {
  actual_received_minor: string;
}

export interface PlanQuotaInput {
  window_kind: PlanWindowKind;
  window_seconds: number;
  quota_fen_cny: string;
  sort_order: number;
}

export interface StorePlanQuota extends PlanQuotaInput {
  id: string;
}

export interface StoreProductInput {
  kind: ProductKind;
  name: string;
  description: string;
  price_currency: StoreCurrency;
  price_minor: string;
  duration_seconds: number | null;
  group_ids: string[];
  sort_order: number;
  enabled: boolean;
  balance: BalanceProductInput | null;
  quotas: PlanQuotaInput[];
}

export interface StoreProduct {
  id: string;
  kind: ProductKind;
  name: string;
  description: string;
  price_currency: StoreCurrency;
  price_minor: string;
  duration_seconds: number | null;
  group_ids: string[];
  sort_order: number;
  enabled: boolean;
  created_at: string;
  updated_at: string;
  balance: StoreBalanceProduct | null;
  quotas: StorePlanQuota[];
}

export interface PaymentChannelInput {
  kind: PaymentChannelKind;
  name: string;
  mode: PaymentChannelMode;
  endpoint: string | null;
  icon_kind: PaymentChannelIconKind;
  icon_value: string | null;
  config_secret: string | null;
  sort_order: number;
  enabled: boolean;
}

export type PaymentChannelUpdate = Partial<PaymentChannelInput>;

export interface StorePaymentChannel {
  id: string;
  kind: PaymentChannelKind;
  name: string;
  mode: PaymentChannelMode;
  endpoint: string | null;
  icon_kind: PaymentChannelIconKind;
  icon_value: string | null;
  sort_order: number;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export interface StoreCatalog {
  products: StoreProduct[];
  payment_channels: StorePaymentChannel[];
  settings: StoreSettings;
}

export interface StoreExchangeRate {
  base: "USD";
  quote: "CNY";
  cny_per_usd: string;
  source_updated_at: string;
  refreshed_at: string;
}

export interface CreateStoreOrderInput {
  product_id: string;
  payment_channel_id: string;
  payment_currency: StoreCurrency;
  custom_recharge_minor?: string | null;
}

export interface StoreProductSnapshot {
  id: string;
  kind: ProductKind;
  name: string;
  description: string;
  price_currency: StoreCurrency;
  price_minor: string;
  duration_seconds: number | null;
  group_ids: string[];
  balance: StoreBalanceProduct | null;
  quotas: StorePlanQuota[];
}

export interface StorePaymentChannelSnapshot {
  id: string;
  kind: PaymentChannelKind;
  name: string;
  mode: PaymentChannelMode;
  endpoint: string | null;
  icon_kind: PaymentChannelIconKind;
  icon_value: string | null;
}

export interface StoreOrderQuote {
  version: number;
  product: StoreProductSnapshot;
  balance: StoreBalanceProduct | null;
  payment_channel: StorePaymentChannelSnapshot;
}

export interface StoreOrder {
  id: string;
  order_number: string;
  user_id: string;
  product_id: string;
  product_kind: ProductKind;
  status: StoreOrderStatus;
  payment_channel_id: string;
  payment_currency: StoreCurrency;
  payment_minor: string;
  cny_per_usd: string;
  rate_source_updated_at: string;
  quote: StoreOrderQuote;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
  cancelled_at: string | null;
}

export interface StorePlanEntitlement {
  id: string;
  user_id: string;
  product_id: string;
  product_name: string;
  starts_at: string;
  ends_at: string;
  cny_per_usd: string;
  group_ids: string[];
  quotas: StorePlanQuota[];
  source_kind: string;
  source_id: string;
}

export type RedemptionRewardInput =
  | { kind: "balance"; currency: StoreCurrency; amount_minor: string }
  | { kind: "plan"; product_id: string };

export interface GenerateRedemptionCodesInput {
  reward: RedemptionRewardInput;
  count: number;
  validity_days: number;
}

export interface RedemptionCodeRecord {
  id: string;
  code_hint: string;
  reward_kind: ProductKind;
  reward: unknown;
  status: RedemptionCodeStatus;
  expires_at: string;
  redeemed_by_user_id: string | null;
  redeemed_at: string | null;
  created_by_user_id: string;
  created_at: string;
}

export interface GeneratedRedemptionCode {
  code: string;
  record: RedemptionCodeRecord;
}

export interface StoreSettings {
  custom_recharge_cny_min_minor: string;
  custom_recharge_cny_max_minor: string;
  custom_recharge_usd_min_minor: string;
  custom_recharge_usd_max_minor: string;
}

export interface DeleteStoreRecordResponse {
  success: boolean;
}

async function storeRequest<T>(path: string, options: RequestInit = {}): Promise<T> {
  const response = await fetch(`${STORE_API_BASE}${path}`, {
    ...options,
    headers: {
      "Content-Type": "application/json",
      ...(options.headers as Record<string, string> | undefined),
    },
    credentials: "include",
  });
  const data = await response.json();
  if (!response.ok) {
    throw new Error(data.error?.message || data.error?.code || "Request failed");
  }
  return data as T;
}

function listPath(path: string, limit: number): string {
  return `${path}?limit=${encodeURIComponent(String(limit))}`;
}

function jsonMutation<T>(path: string, method: "POST" | "PUT", body: unknown): Promise<T> {
  return storeRequest<T>(path, { method, body: JSON.stringify(body) });
}

export const storeApi = {
  getCatalog: () => storeRequest<StoreCatalog>("/catalog"),
  getExchangeRate: () => storeRequest<StoreExchangeRate>("/exchange-rate"),
  getEntitlement: () => storeRequest<StorePlanEntitlement | null>("/entitlement"),
  listOrders: (limit = 100) => storeRequest<StoreOrder[]>(listPath("/orders", limit)),
  createOrder: (input: CreateStoreOrderInput) =>
    jsonMutation<StoreOrder>("/orders", "POST", input),
  redeem: (code: string) =>
    jsonMutation<RedemptionCodeRecord>("/redeem", "POST", { code }),
  admin: {
    listProducts: () => storeRequest<StoreProduct[]>("/admin/products"),
    createProduct: (input: StoreProductInput) =>
      jsonMutation<StoreProduct>("/admin/products", "POST", input),
    updateProduct: (id: string, input: StoreProductInput) =>
      jsonMutation<StoreProduct>(`/admin/products/${encodeURIComponent(id)}`, "PUT", input),
    deleteProduct: (id: string) =>
      storeRequest<DeleteStoreRecordResponse>(`/admin/products/${encodeURIComponent(id)}`, {
        method: "DELETE",
      }),
    listPaymentChannels: () =>
      storeRequest<StorePaymentChannel[]>("/admin/payment-channels"),
    createPaymentChannel: (input: PaymentChannelInput) =>
      jsonMutation<StorePaymentChannel>("/admin/payment-channels", "POST", input),
    updatePaymentChannel: (id: string, input: PaymentChannelUpdate) =>
      jsonMutation<StorePaymentChannel>(
        `/admin/payment-channels/${encodeURIComponent(id)}`,
        "PUT",
        input,
      ),
    deletePaymentChannel: (id: string) =>
      storeRequest<DeleteStoreRecordResponse>(
        `/admin/payment-channels/${encodeURIComponent(id)}`,
        { method: "DELETE" },
      ),
    listOrders: (limit = 100) =>
      storeRequest<StoreOrder[]>(listPath("/admin/orders", limit)),
    completeOrder: (id: string) =>
      storeRequest<StoreOrder>(`/admin/orders/${encodeURIComponent(id)}/complete`, {
        method: "POST",
      }),
    cancelOrder: (id: string) =>
      storeRequest<StoreOrder>(`/admin/orders/${encodeURIComponent(id)}/cancel`, {
        method: "POST",
      }),
    listRedemptionCodes: (limit = 100) =>
      storeRequest<RedemptionCodeRecord[]>(listPath("/admin/redemption-codes", limit)),
    generateRedemptionCodes: (input: GenerateRedemptionCodesInput) =>
      jsonMutation<GeneratedRedemptionCode[]>("/admin/redemption-codes", "POST", input),
    getSettings: () => storeRequest<StoreSettings>("/admin/settings"),
    updateSettings: (input: StoreSettings) =>
      jsonMutation<StoreSettings>("/admin/settings", "PUT", input),
  },
};
