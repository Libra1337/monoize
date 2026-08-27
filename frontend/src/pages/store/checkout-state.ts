import type { CreateStoreOrderInput } from "@/lib/store-api";

const CHECKOUT_STATE_KEY = "lynshen.store.pending-checkout.v1";

export interface PendingCheckoutState {
  fingerprint: string;
  orderId: string | null;
  orderIdempotencyKey: string;
  attemptIdempotencyKey: string;
}

export function checkoutFingerprint(userId: string, input: CreateStoreOrderInput): string {
  return JSON.stringify([
    userId,
    input.product_id,
    input.payment_channel_id,
    input.payment_currency,
    input.custom_recharge_minor ?? null,
  ]);
}

export function loadPendingCheckout(storage: Storage): PendingCheckoutState | null {
  try {
    const parsed = JSON.parse(storage.getItem(CHECKOUT_STATE_KEY) ?? "null") as Partial<PendingCheckoutState> | null;
    if (
      !parsed
      || typeof parsed.fingerprint !== "string"
      || typeof parsed.orderIdempotencyKey !== "string"
      || typeof parsed.attemptIdempotencyKey !== "string"
      || !(typeof parsed.orderId === "string" || parsed.orderId === null)
    ) {
      return null;
    }
    return parsed as PendingCheckoutState;
  } catch {
    return null;
  }
}

export function preparePendingCheckout(
  storage: Storage,
  fingerprint: string,
): PendingCheckoutState {
  const existing = loadPendingCheckout(storage);
  if (existing?.fingerprint === fingerprint) return existing;
  const pending: PendingCheckoutState = {
    fingerprint,
    orderId: null,
    orderIdempotencyKey: crypto.randomUUID(),
    attemptIdempotencyKey: crypto.randomUUID(),
  };
  savePendingCheckout(storage, pending);
  return pending;
}

export function savePendingCheckout(storage: Storage, pending: PendingCheckoutState): void {
  storage.setItem(CHECKOUT_STATE_KEY, JSON.stringify(pending));
}

export function rotatePendingAttempt(storage: Storage, pending: PendingCheckoutState): void {
  pending.attemptIdempotencyKey = crypto.randomUUID();
  savePendingCheckout(storage, pending);
}

export function isDefiniteAttemptFailure(code: string): boolean {
  return code === "payment_configuration_unavailable" || code === "payment_provider_rejected";
}

export function isPaymentPollingTerminal(paymentState: string): boolean {
  return paymentState === "paid" || paymentState === "refunded" || paymentState === "closed";
}

export function clearPendingCheckout(storage: Storage, orderId?: string): void {
  const existing = loadPendingCheckout(storage);
  if (!orderId || existing?.orderId === orderId) storage.removeItem(CHECKOUT_STATE_KEY);
}
