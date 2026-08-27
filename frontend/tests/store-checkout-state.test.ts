import { describe, expect, test } from "bun:test";
import {
  checkoutFingerprint,
  isDefiniteAttemptFailure,
  isPaymentPollingTerminal,
  preparePendingCheckout,
  savePendingCheckout,
} from "../src/pages/store/checkout-state";

class MemoryStorage implements Storage {
  private values = new Map<string, string>();
  get length() { return this.values.size; }
  clear() { this.values.clear(); }
  getItem(key: string) { return this.values.get(key) ?? null; }
  key(index: number) { return [...this.values.keys()][index] ?? null; }
  removeItem(key: string) { this.values.delete(key); }
  setItem(key: string, value: string) { this.values.set(key, value); }
}

describe("Store checkout idempotency state", () => {
  test("reuses both keys for the same request after the order is bound", () => {
    const storage = new MemoryStorage();
    const fingerprint = checkoutFingerprint("user-1", {
      product_id: "product-1",
      payment_channel_id: "stripe-1",
      payment_currency: "CNY",
      custom_recharge_minor: null,
    });
    const first = preparePendingCheckout(storage, fingerprint);
    first.orderId = "order-1";
    savePendingCheckout(storage, first);

    expect(preparePendingCheckout(storage, fingerprint)).toEqual(first);
  });

  test("creates new keys when the canonical request changes", () => {
    const storage = new MemoryStorage();
    const first = preparePendingCheckout(storage, "request-a");
    const second = preparePendingCheckout(storage, "request-b");

    expect(second.orderIdempotencyKey).not.toBe(first.orderIdempotencyKey);
    expect(second.attemptIdempotencyKey).not.toBe(first.attemptIdempotencyKey);
  });

  test("treats paid payment as terminal for browser polling", () => {
    expect(isPaymentPollingTerminal("paid")).toBe(true);
    expect(isPaymentPollingTerminal("refunded")).toBe(true);
    expect(isPaymentPollingTerminal("closed")).toBe(true);
    expect(isPaymentPollingTerminal("unpaid")).toBe(false);
  });

  test("rotates attempt keys only for definite checkout failures", () => {
    expect(isDefiniteAttemptFailure("payment_configuration_unavailable")).toBe(true);
    expect(isDefiniteAttemptFailure("payment_provider_rejected")).toBe(true);
    expect(isDefiniteAttemptFailure("payment_provider_ambiguous")).toBe(false);
    expect(isDefiniteAttemptFailure("internal_error")).toBe(false);
  });
});
