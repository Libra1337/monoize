import type { StoreCurrency } from "@/lib/store-money";

const STORE_API_BASE = "/api/dashboard/store";

export type ProductKind = "balance" | "plan";
export type PaymentAdapterKind = "alipay" | "wechat" | "stripe" | "http";
export type OfficialPaymentAdapterKind = Exclude<PaymentAdapterKind, "http">;
export type PaymentChannelIconKind = "builtin" | "url" | "upload";
export type PlanWindowKind = "5h" | "12h" | "day" | "week" | "month" | "custom";
export type StorePaymentState = "unpaid" | "paid" | "refund_pending" | "refunded" | "closed";
export type StoreFulfillmentState = "pending" | "fulfilled" | "failed";
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
  adapter_kind: PaymentAdapterKind;
  name: string;
  icon_kind: PaymentChannelIconKind;
  icon_value: string | null;
  sort_order: number;
  enabled: boolean;
}

export type PaymentChannelUpdate = Partial<PaymentChannelInput> & { expected_revision: number };

export interface StorePaymentChannel {
  id: string;
  adapter_kind: PaymentAdapterKind;
  name: string;
  icon_kind: PaymentChannelIconKind;
  icon_value: string | null;
  sort_order: number;
  enabled: boolean;
  revision: number;
  created_at: string;
  updated_at: string;
}

export type PaymentCredentialInput =
  | {
      adapter_kind: "stripe";
      secret_key: string;
      publishable_key: string;
      webhook_signing_secret: string;
      api_version: string;
      account_id: string;
      live_mode: boolean;
    }
  | {
      adapter_kind: "alipay";
      app_id: string;
      seller_id: string;
      merchant_private_key_pem: string;
      alipay_public_key_pem: string;
      environment: "production" | "sandbox";
    }
  | {
      adapter_kind: "wechat";
      merchant_id: string;
      app_id: string;
      api_v3_key: string;
      merchant_certificate_serial: string;
      merchant_private_key_pem: string;
    };

export type PaymentCredentialPayload =
  | Omit<Extract<PaymentCredentialInput, { adapter_kind: "stripe" }>, "adapter_kind">
  | Omit<Extract<PaymentCredentialInput, { adapter_kind: "alipay" }>, "adapter_kind">
  | Omit<Extract<PaymentCredentialInput, { adapter_kind: "wechat" }>, "adapter_kind">;

export interface StoreReauthGrant {
  token: string;
  scope: "credential_update";
  expires_at: string;
}

export interface SavedPaymentCredential {
  id: string;
  channel_id: string;
  adapter_kind: OfficialPaymentAdapterKind;
  account_identity_digest: string;
  status: "active";
  created_at: string;
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
  adapter_kind: PaymentAdapterKind;
  name: string;
  icon_kind: PaymentChannelIconKind;
  icon_value: string | null;
}

export interface StoreOrderQuote {
  version: number;
  product: StoreProductSnapshot;
  payment_channel: StorePaymentChannelSnapshot;
  rate: {
    decimal: string;
    numerator: string;
    denominator: string;
    source_updated_at: string;
    refreshed_at: string;
  };
}

export interface StoreOrder {
  id: string;
  order_number: string;
  user_id: string;
  product_id: string;
  product_kind: ProductKind;
  payment_state: StorePaymentState;
  fulfillment_state: StoreFulfillmentState;
  dispute_state: "none" | "open" | "won" | "lost";
  payment_hold: boolean;
  payment_channel_id: string;
  payment_currency: StoreCurrency;
  payment_minor: string;
  cny_per_usd: string;
  rate_numerator: string;
  rate_denominator: string;
  quote: StoreOrderQuote;
  contract_version: number;
  state_revision: number;
  expires_at: string;
  created_at: string;
  updated_at: string;
}

export interface StorePaymentAttempt {
  id: string;
  order_id: string;
  channel_id: string;
  adapter_kind: PaymentAdapterKind;
  credential_version_id: string;
  merchant_account_identity: string;
  expected_payment_method: string | null;
  payment_contract_version: number;
  state: "created" | "presented" | "expired" | "failed" | "paid";
  failure_kind: "configuration_unavailable" | "provider_rejected" | null;
  idempotency_key: string;
  provider_object_id: string | null;
  action: StoreCheckoutAction | null;
  provider_expires_at: string | null;
  created_at: string;
  updated_at: string;
}

export type StoreCheckoutAction =
  | { kind: "redirect"; url: string; expires_at: string }
  | { kind: "qr"; payload: string; expires_at: string }
  | { kind: "form"; action: string; fields: Array<[string, string]>; expires_at: string };

export interface StoreCheckoutResponse {
  attempt: StorePaymentAttempt;
  action: StoreCheckoutAction;
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

export class StoreApiError extends Error {
  readonly code: string;
  readonly status: number;

  constructor(
    message: string,
    code: string,
    status: number,
  ) {
    super(message);
    this.name = "StoreApiError";
    this.code = code;
    this.status = status;
  }
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
    throw new StoreApiError(
      data.error?.message || data.error?.code || "Request failed",
      data.error?.code || "request_failed",
      response.status,
    );
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
  getOrder: (id: string) => storeRequest<StoreOrder>(`/orders/${encodeURIComponent(id)}`),
  createOrder: (input: CreateStoreOrderInput, idempotencyKey: string) =>
    storeRequest<StoreOrder>("/orders", {
      method: "POST",
      headers: { "Idempotency-Key": idempotencyKey },
      body: JSON.stringify(input),
    }),
  createPaymentAttempt: (
    orderId: string,
    idempotencyKey: string,
    expectedPaymentMethod: string | null,
  ) =>
    storeRequest<StoreCheckoutResponse>(`/orders/${encodeURIComponent(orderId)}/attempts`, {
      method: "POST",
      headers: { "Idempotency-Key": idempotencyKey },
      body: JSON.stringify({ expected_payment_method: expectedPaymentMethod }),
    }),
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
    createReauthGrant: (currentPassword: string) =>
      jsonMutation<StoreReauthGrant>("/admin/reauth", "POST", {
        current_password: currentPassword,
        scope: "credential_update",
      }),
    replacePaymentCredential: (
      id: string,
      credential: PaymentCredentialPayload,
      reauthToken: string,
    ) =>
      storeRequest<SavedPaymentCredential>(
        `/admin/payment-channels/${encodeURIComponent(id)}/credential`,
        {
          method: "PUT",
          headers: { "X-Store-Reauth-Token": reauthToken },
          body: JSON.stringify(credential),
        },
      ),
    uploadIcon: async (file: File) => {
      const formData = new FormData();
      formData.append("file", file);
      const response = await fetch(`${STORE_API_BASE}/admin/icons`, {
        method: "POST",
        body: formData,
        credentials: "include",
      });
      const data = await response.json();
      if (!response.ok) {
        throw new Error(data.error?.message || data.error?.code || "Request failed");
      }
      return data as { url: string };
    },
    listOrders: (limit = 100) =>
      storeRequest<StoreOrder[]>(listPath("/admin/orders", limit)),
    listRedemptionCodes: (limit = 100) =>
      storeRequest<RedemptionCodeRecord[]>(listPath("/admin/redemption-codes", limit)),
    generateRedemptionCodes: (input: GenerateRedemptionCodesInput) =>
      jsonMutation<GeneratedRedemptionCode[]>("/admin/redemption-codes", "POST", input),
    getSettings: () => storeRequest<StoreSettings>("/admin/settings"),
    updateSettings: (input: StoreSettings) =>
      jsonMutation<StoreSettings>("/admin/settings", "PUT", input),
  },
};
