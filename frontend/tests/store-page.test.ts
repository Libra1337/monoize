import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";

const readSource = (relativePath: string) => {
  const url = new URL(relativePath, import.meta.url);
  return existsSync(url) ? readFileSync(url, "utf8") : "";
};

const storeSource = readSource("../src/pages/store/index.tsx");
const tabsSource = readSource("../src/pages/store/store-tabs.tsx");
const paymentSource = readSource("../src/pages/store/payment-methods.tsx");
const summarySource = readSource("../src/pages/store/order-summary.tsx");
const skeletonSource = readSource("../src/pages/store/store-skeleton.tsx");
const redemptionSource = readSource("../src/pages/store/redemption-panel.tsx");
const checkoutStateSource = readSource("../src/pages/store/checkout-state.ts");
const ordersSource = readSource("../src/pages/orders.tsx");
const moneySource = readSource("../src/lib/store-money.ts");
const zhSource = readSource("../src/locales/zh.json");
const currencySource = readSource("../src/hooks/use-store-currency.tsx");
const appSource = readSource("../src/App.tsx");

describe("Store user pages", () => {
  test("renders three Store tabs with one animated shared indicator", () => {
    expect(tabsSource).toContain('"balance"');
    expect(tabsSource).toContain('"plan"');
    expect(tabsSource).toContain('"redeem"');
    expect(tabsSource).toContain("layoutId");
    expect(tabsSource).toContain('aria-selected={activeTab === tab}');
  });

  test("uses 16px cards, 12px controls, and a stable summary block", () => {
    expect(storeSource).toContain("rounded-2xl");
    expect(tabsSource).toContain("rounded-xl");
    expect(summarySource).toContain("min-h-");
  });

  test("keeps payment methods in a separate full-width bottom section", () => {
    expect(paymentSource).toContain("w-full");
    expect(paymentSource).toContain("SiAlipay");
    expect(paymentSource).toContain("SiWechat");
    expect(storeSource.indexOf("<PaymentMethods")).toBeGreaterThan(
      storeSource.indexOf("<OrderSummary"),
    );
  });

  test("replaces a failed custom payment image with the channel icon fallback", () => {
    expect(paymentSource).toContain("onError");
    expect(paymentSource).toContain("setImageFailed(true)");
    expect(paymentSource).toContain("key={channel.icon_value ?? channel.id}");
  });

  test("does not mount payment methods or a summary for redemption", () => {
    expect(storeSource).toContain('activeTab !== "redeem"');
    expect(storeSource).toContain("<RedemptionPanel");
  });

  test("uses one CNY or USD state for balance and plan presentation", () => {
    expect(storeSource).toContain("useStoreCurrency()");
    expect(storeSource).not.toContain('useState<StoreCurrency>("CNY")');
    expect(storeSource).toContain("currency={currency}");
    expect(storeSource).toContain("onCurrencyChange={setCurrency}");
    expect(currencySource).toContain("StoreCurrencyProvider");
    expect(currencySource).toContain('STORE_CURRENCY_STORAGE_KEY = "monoize-display-currency-v1"');
    expect(currencySource).toContain("localStorage.getItem");
    expect(currencySource).toContain("localStorage.setItem");
    expect(appSource).toContain("<StoreCurrencyProvider>");
    expect(moneySource).toContain("formatNanoUsd");
    expect(moneySource).toContain("formatPlanQuota");
  });

  test("validates non-empty custom amounts against catalog currency bounds", () => {
    expect(storeSource).toContain("validateCustomAmount");
    expect(storeSource).toContain("catalog.data?.settings");
    expect(storeSource).toContain("customAmountInvalid");
    expect(summarySource).toContain("customAmountInvalid");
  });

  test("adds separately converted recharge and bonus values in cards and summary", () => {
    expect(storeSource).toContain("addMinor");
    expect(summarySource).toContain("addMinor");
    expect(storeSource).not.toContain("product.balance.actual_received_minor");
    expect(summarySource).not.toContain("product.balance.actual_received_minor");
  });

  test("uses SWR skeletons and optimistic rollback for Store mutations", () => {
    expect(storeSource).toContain("useSWR");
    expect(storeSource).toContain("<StoreSkeleton");
    expect(storeSource).toContain("optimisticData");
    expect(storeSource).toContain("rollbackOnError: true");
    expect(skeletonSource).toContain("Skeleton");
  });

  test("starts the persisted payment attempt and follows redirect checkout actions", () => {
    expect(storeSource).toContain("storeApi.createPaymentAttempt");
    expect(storeSource).toContain("preparePendingCheckout");
    expect(checkoutStateSource).toContain("crypto.randomUUID()");
    expect(storeSource).toContain("window.sessionStorage");
    expect(storeSource).toContain("window.location.assign(checkout.action.url)");
    expect(storeSource).toContain('stripe: "card"');
    expect(storeSource).toContain('? "mobile_web" : "computer_web"');
    expect(storeSource).toContain('validatedChannel.adapter_kind === "wechat"');
    expect(storeSource).toContain('(max-width: 767px)');
    expect(storeSource).toContain('? "h5" : "native"');
  });

  test("reacts to viewport changes and submits only a currently compatible Channel", () => {
    expect(storeSource).toContain("filterCompatiblePaymentChannels");
    expect(storeSource).toContain("compatibleChannels");
    expect(storeSource).toContain('addEventListener("change"');
    expect(storeSource).toContain('removeEventListener("change"');
    expect(storeSource).toContain("window.matchMedia(MOBILE_CHECKOUT_QUERY).matches");
    expect(storeSource).toContain("validatedChannel");
    expect(storeSource).toContain("channels={compatibleChannels}");
    expect(paymentSource).not.toContain("channels.filter");
  });

  test("polls a pending payment every two seconds and revalidates account data", () => {
    expect(storeSource).toContain("storeApi.getOrder(pollingOrderId)");
    expect(storeSource).toContain("2_000");
    expect(storeSource).toContain("refreshUser()");
    expect(storeSource).toContain("entitlement.mutate()");
    expect(storeSource).toContain("shouldContinueCheckoutPolling({");
    expect(storeSource).toContain("paymentState: order.payment_state");
    expect(ordersSource).toContain("isPaymentPollingTerminal(current.payment_state)");
  });

  test("renders a scannable SVG for WeChat Native checkout", () => {
    expect(storeSource).toContain("QRCodeSVG");
    expect(storeSource).toContain("value={qrAction.payload}");
  });

  test("rotates an attempt key only after a definite provider failure", () => {
    expect(storeSource).toContain("rotatePendingAttemptAfterFailure(");
    expect(storeSource).not.toContain("rotatePendingAttempt(");
  });

  test("renders an optimistic redemption status before the API finishes", () => {
    expect(storeSource).toContain("REDEMPTION_STATUS_KEY");
    expect(storeSource).toContain('optimisticData: { state: "redeeming"');
    expect(storeSource).toContain("redeeming={redeeming}");
    expect(redemptionSource).toContain('role="status"');
  });

  test("keeps order history on a separate SWR page with a detail dialog", () => {
    expect(ordersSource).toContain("useSWR");
    expect(ordersSource).toContain("storeApi.listOrders");
    expect(ordersSource).toContain("Dialog");
    expect(ordersSource).toContain("Skeleton");
    expect(ordersSource).toContain("storeApi.getOrder(selectedOrder.id)");
    expect(ordersSource).toContain("2_000");
  });

  test("uses the exact Simplified Chinese wording 实得", () => {
    expect(zhSource).toContain("实得");
    expect(zhSource).toContain("实得金额");
    expect(zhSource).not.toContain("实到");
  });
});
