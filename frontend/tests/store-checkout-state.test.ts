import { describe, expect, test } from "bun:test";
import {
  checkoutFingerprint,
  isDefiniteAttemptFailure,
  isPaymentPollingTerminal,
  rotatePendingAttemptAfterFailure,
  preparePendingCheckout,
  savePendingCheckout,
  shouldContinueCheckoutPolling,
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

  test("stops polling after payment succeeds while fulfillment remains pending", () => {
    expect(shouldContinueCheckoutPolling({
      paymentState: "paid",
      fulfillmentState: "pending",
      expiresAt: "2099-01-01T00:00:00.000Z",
    }, Date.parse("2026-08-28T00:00:00.000Z"))).toBe(false);
  });

  test("rotates attempt keys only for definite checkout failures", () => {
    expect(isDefiniteAttemptFailure("payment_configuration_unavailable")).toBe(true);
    expect(isDefiniteAttemptFailure("payment_provider_rejected")).toBe(true);
    expect(isDefiniteAttemptFailure("payment_provider_ambiguous")).toBe(false);
    expect(isDefiniteAttemptFailure("internal_error")).toBe(false);
  });

  test("rotates an attempt key only for the two allowed failures after order creation", () => {
    for (const code of ["payment_configuration_unavailable", "payment_provider_rejected"]) {
      const storage = new MemoryStorage();
      const pending = preparePendingCheckout(storage, code);
      pending.orderId = "order-1";
      savePendingCheckout(storage, pending);
      const previousKey = pending.attemptIdempotencyKey;

      expect(rotatePendingAttemptAfterFailure(storage, pending, code)).toBe(true);
      expect(pending.attemptIdempotencyKey).not.toBe(previousKey);
    }
  });

  test("retains the attempt key for network, internal, ambiguous, and unrecognized failures", () => {
    for (const code of [null, "internal_error", "payment_provider_ambiguous", "unexpected_error"]) {
      const storage = new MemoryStorage();
      const pending = preparePendingCheckout(storage, code ?? "network");
      pending.orderId = "order-1";
      savePendingCheckout(storage, pending);
      const previousKey = pending.attemptIdempotencyKey;

      expect(rotatePendingAttemptAfterFailure(storage, pending, code)).toBe(false);
      expect(pending.attemptIdempotencyKey).toBe(previousKey);
    }
  });

  test("retains the attempt key before an order ID exists", () => {
    const storage = new MemoryStorage();
    const pending = preparePendingCheckout(storage, "request-a");
    const previousKey = pending.attemptIdempotencyKey;

    expect(rotatePendingAttemptAfterFailure(
      storage,
      pending,
      "payment_provider_rejected",
    )).toBe(false);
    expect(pending.attemptIdempotencyKey).toBe(previousKey);
  });
});
