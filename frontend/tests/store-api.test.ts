import { afterEach, describe, expect, test } from "bun:test";
import { storeApi } from "../src/lib/store-api";

const originalFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = originalFetch;
});

describe("Store API transport", () => {
  test("loads the catalog with the dashboard session cookie", async () => {
    let requestUrl = "";
    let credentials: RequestCredentials | undefined;
    globalThis.fetch = (async (input, init) => {
      requestUrl = String(input);
      credentials = init?.credentials;
      return Response.json({ products: [], payment_channels: [] });
    }) as typeof fetch;

    await storeApi.getCatalog();

    expect(requestUrl).toBe("/api/dashboard/store/catalog");
    expect(credentials).toBe("include");
  });

  test("creates an order with the exact JSON field names", async () => {
    let body = "";
    let idempotencyKey = "";
    globalThis.fetch = (async (_input, init) => {
      body = String(init?.body);
      idempotencyKey = new Headers(init?.headers).get("Idempotency-Key") ?? "";
      return Response.json({ id: "order-1" }, { status: 201 });
    }) as typeof fetch;

    await storeApi.createOrder(
      {
        product_id: "product-1",
        payment_channel_id: "channel-1",
        payment_currency: "CNY",
        custom_recharge_minor: "1200",
      },
      "checkout-1",
    );

    expect(JSON.parse(body)).toEqual({
      product_id: "product-1",
      payment_channel_id: "channel-1",
      payment_currency: "CNY",
      custom_recharge_minor: "1200",
    });
    expect(idempotencyKey).toBe("checkout-1");
  });

  test("creates a payment attempt with an idempotency key and returns its action", async () => {
    let requestUrl = "";
    let body = "";
    let idempotencyKey = "";
    globalThis.fetch = (async (input, init) => {
      requestUrl = String(input);
      body = String(init?.body);
      idempotencyKey = new Headers(init?.headers).get("Idempotency-Key") ?? "";
      return Response.json({
        attempt: { id: "attempt-1", state: "presented" },
        action: {
          kind: "redirect",
          url: "https://checkout.stripe.com/c/pay_1",
          expires_at: "2026-08-27T18:00:00Z",
        },
      }, { status: 201 });
    }) as typeof fetch;

    const result = await storeApi.createPaymentAttempt("order-1", "attempt-key-1", "card");

    expect(requestUrl).toBe("/api/dashboard/store/orders/order-1/attempts");
    expect(JSON.parse(body)).toEqual({ expected_payment_method: "card" });
    expect(idempotencyKey).toBe("attempt-key-1");
    expect(result.action.kind).toBe("redirect");
  });

  test("maps user and admin operations to the Store route surface", async () => {
    const requests: Array<{ url: string; method: string }> = [];
    globalThis.fetch = (async (input, init) => {
      requests.push({ url: String(input), method: init?.method ?? "GET" });
      return Response.json([]);
    }) as typeof fetch;

    await storeApi.getExchangeRate();
    await storeApi.listOrders(25);
    await storeApi.redeem("ABCD-EFGH-IJKL-MNOP");
    await storeApi.admin.listProducts();
    await storeApi.admin.listPaymentChannels();
    await storeApi.admin.listOrders(50);
    await storeApi.admin.listRedemptionCodes(10);
    await storeApi.admin.getSettings();

    expect(requests).toEqual([
      { url: "/api/dashboard/store/exchange-rate", method: "GET" },
      { url: "/api/dashboard/store/orders?limit=25", method: "GET" },
      { url: "/api/dashboard/store/redeem", method: "POST" },
      { url: "/api/dashboard/store/admin/products", method: "GET" },
      { url: "/api/dashboard/store/admin/payment-channels", method: "GET" },
      { url: "/api/dashboard/store/admin/orders?limit=50", method: "GET" },
      { url: "/api/dashboard/store/admin/redemption-codes?limit=10", method: "GET" },
      { url: "/api/dashboard/store/admin/settings", method: "GET" },
    ]);
  });

  test("does not expose manual order completion or cancellation", () => {
    expect("completeOrder" in storeApi.admin).toBe(false);
    expect("cancelOrder" in storeApi.admin).toBe(false);
  });

  test("maps Admin order detail query and close to exact contracts", async () => {
    const requests: Array<{ url: string; method: string; body: unknown }> = [];
    globalThis.fetch = (async (input, init) => {
      requests.push({
        url: String(input),
        method: init?.method ?? "GET",
        body: init?.body ? JSON.parse(String(init.body)) : null,
      });
      return Response.json({ order: {}, attempts: [], refunds: [] });
    }) as typeof fetch;

    await storeApi.admin.getOrderDetail("order /1");
    await storeApi.admin.queryOrder("order /1", "attempt /1");
    await storeApi.admin.closeOrder("order /1", "attempt /1");

    expect(requests).toEqual([
      {
        url: "/api/dashboard/store/admin/orders/order%20%2F1",
        method: "GET",
        body: null,
      },
      {
        url: "/api/dashboard/store/admin/orders/order%20%2F1/query",
        method: "POST",
        body: { attempt_id: "attempt /1" },
      },
      {
        url: "/api/dashboard/store/admin/orders/order%20%2F1/close",
        method: "POST",
        body: { attempt_id: "attempt /1" },
      },
    ]);
  });

  test("maps Admin refund create detail and query with required headers", async () => {
    const requests: Array<{
      url: string;
      method: string;
      body: unknown;
      reauth: string | null;
      idempotency: string | null;
    }> = [];
    globalThis.fetch = (async (input, init) => {
      const headers = new Headers(init?.headers);
      requests.push({
        url: String(input),
        method: init?.method ?? "GET",
        body: init?.body ? JSON.parse(String(init.body)) : null,
        reauth: headers.get("X-Store-Reauth-Token"),
        idempotency: headers.get("Idempotency-Key"),
      });
      return Response.json({ id: "refund-1" });
    }) as typeof fetch;

    await storeApi.admin.createRefund("order /1", "refund-key", "refund-grant");
    await storeApi.admin.getRefund("order /1", "refund /1");
    await storeApi.admin.queryRefund("order /1", "refund /1", "refund-grant");

    expect(requests).toEqual([
      {
        url: "/api/dashboard/store/admin/orders/order%20%2F1/refunds",
        method: "POST",
        body: {},
        reauth: "refund-grant",
        idempotency: "refund-key",
      },
      {
        url: "/api/dashboard/store/admin/orders/order%20%2F1/refunds/refund%20%2F1",
        method: "GET",
        body: null,
        reauth: null,
        idempotency: null,
      },
      {
        url: "/api/dashboard/store/admin/orders/order%20%2F1/refunds/refund%20%2F1/query",
        method: "POST",
        body: {},
        reauth: "refund-grant",
        idempotency: null,
      },
    ]);
  });
});
