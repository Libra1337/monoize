import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { createInstance } from "i18next";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { StoreAdminTabs } from "../src/pages/store-admin/store-admin-tabs";
import { canCloseAttempt } from "../src/pages/store-admin/order-actions";
import { disabledOptimisticChannel } from "../src/pages/store-admin/channel-state";
import type { StoreOrder, StorePaymentAttempt, StorePaymentChannel } from "../src/lib/store-api";

const testI18n = createInstance();
await testI18n.init({
  lng: "en",
  resources: {
    en: {
      translation: {
        store: {
          admin: {
            tabsLabel: "Store management sections",
            tabs: {
              products: "Products",
              channels: "Payment Channels",
              orders: "Orders",
              redemptions: "Redemption Codes",
            },
          },
        },
      },
    },
  },
});

const readSource = (relativePath: string) => {
  const url = new URL(relativePath, import.meta.url);
  return existsSync(url) ? readFileSync(url, "utf8") : "";
};

const pageSource = readSource("../src/pages/store-admin/index.tsx");
const tabsSource = readSource("../src/pages/store-admin/store-admin-tabs.tsx");
const productSource = readSource("../src/pages/store-admin/product-dialog.tsx");
const channelSource = readSource("../src/pages/store-admin/channel-dialog.tsx");
const redemptionSource = readSource("../src/pages/store-admin/redemption-dialog.tsx");
const panelsSource = readSource("../src/pages/store-admin/admin-panels.tsx");
const orderDialogSource = readSource("../src/pages/store-admin/order-dialog.tsx");
const governanceDialogsSource = readSource("../src/pages/store-admin/governance-dialogs.tsx");
const apiSource = readSource("../src/lib/store-api.ts");

describe("Store admin page", () => {
  test("renders four animated administration tabs", () => {
    expect(tabsSource).toContain('"products"');
    expect(tabsSource).toContain('"channels"');
    expect(tabsSource).toContain('"orders"');
    expect(tabsSource).toContain('"redemptions"');
    expect(tabsSource).toContain("layoutId");
    expect(tabsSource).toContain('role="tablist"');
  });

  test("server-renders four accessible tab controls", () => {
    const html = renderToStaticMarkup(createElement(
      I18nextProvider,
      { i18n: testI18n },
      createElement(StoreAdminTabs, {
        activeTab: "products",
        onTabChange: () => undefined,
      }),
    ));
    expect(html.match(/role="tab"/g)).toHaveLength(4);
    expect(html).toContain('aria-selected="true"');
    expect(html).toContain('aria-selected="false"');
  });

  test("derives balance actual received from recharge and bonus", () => {
    expect(productSource).toContain("recharge_minor");
    expect(productSource).toContain("bonus_minor");
    expect(productSource).toContain("addMinor");
    expect(productSource).toContain("store.admin.products.actualReceived");
  });

  test("supports multiple plan quota windows including custom whole hours", () => {
    expect(productSource).toContain("quotas.map");
    expect(productSource).toContain('"custom_hours"');
    expect(productSource).toContain("window_hours");
    expect(productSource).toContain("Number.isInteger");
    expect(productSource).toContain("quota_minor_cny");
  });

  test("supports official and HTTP payment adapters with URL or uploaded icons", () => {
    expect(channelSource).toContain('"alipay"');
    expect(channelSource).toContain('"wechat"');
    expect(channelSource).toContain('"stripe"');
    expect(channelSource).toContain('"http"');
    expect(channelSource).not.toContain("config_secret");
    expect(channelSource).toContain('type="file"');
    expect(channelSource).toContain("uploadIcon");
    expect(channelSource).toContain("onError");
    expect(panelsSource).toContain('channel.icon_kind !== "builtin"');
    expect(apiSource).toContain('const STORE_API_BASE = "/api/dashboard/store"');
    expect(apiSource).toContain('`${STORE_API_BASE}/admin/icons`');
  });

  test("shows configured and effective Channel availability separately", () => {
    for (const field of [
      "effective_available",
      "unavailable_reasons",
      "supported_currencies",
      "amount_limits",
      "checkout_action_kinds",
    ]) {
      expect(apiSource).toContain(field);
    }
    expect(panelsSource).toContain("effective_available");
    expect(panelsSource).toContain("unavailable_reasons");
    expect(panelsSource).toContain(".sort(");
    expect(panelsSource).toContain("store.admin.channelAvailability.configuredState");
    expect(panelsSource).toContain("store.admin.channelAvailability.effectiveState");
    expect(panelsSource).toContain("store.admin.channelAvailability.unavailableReasons");
    for (const locale of ["en", "zh", "zh-TW", "ja"]) {
      const source = readSource(`../src/locales/${locale}.json`);
      expect(source).toContain('"configuredState"');
      expect(source).toContain('"effectiveState"');
      expect(source).toContain('"unavailableReasons"');
    }
  });

  test("manages privacy records and official Channel readiness in resilient dialogs", () => {
    for (const method of [
      "listPrivacyRecords",
      "createPrivacyRecord",
      "getChannelReadiness",
      "putChannelReadiness",
    ]) {
      expect(apiSource).toContain(method);
    }
    expect(pageSource).toContain("PrivacyRecordsDialog");
    expect(pageSource).toContain("ChannelReadinessDialog");
    expect(panelsSource).toContain('channel.adapter_kind !== "http"');
    expect(governanceDialogsSource).toContain("useSWR");
    expect(governanceDialogsSource).toContain("Skeleton");
    expect(governanceDialogsSource).toContain("optimisticData");
    expect(governanceDialogsSource).toContain("rollbackOnError: true");
    expect(governanceDialogsSource).toContain("supported_currencies");
    expect(governanceDialogsSource).toContain("checkout_action_kinds");
    expect(governanceDialogsSource).not.toContain("active_credential_digest:");
    for (const locale of ["en", "zh", "zh-TW", "ja"]) {
      const source = readSource(`../src/locales/${locale}.json`);
      expect(source).toContain('"privacyRecords"');
      expect(source).toContain('"readiness"');
    }
  });

  test("replaces official credentials through scoped reauthentication without prefilling secrets", () => {
    expect(apiSource).toContain("createReauthGrant");
    expect(apiSource).toContain("replacePaymentCredential");
    expect(apiSource).toContain('"X-Store-Reauth-Token"');
    expect(apiSource).toContain("expected_revision");
    expect(channelSource).toContain("onSaveCredential");
    expect(channelSource).toContain('type="password"');
    expect(channelSource).toContain("merchant_private_key_pem");
    expect(channelSource).toContain("webhook_signing_secret");
    expect(channelSource).toContain("api_v3_key");
    expect(channelSource).toContain("platform_certificate_serial");
    expect(channelSource).toContain("platform_public_key_pem");
    expect(apiSource).toContain("platform_certificate_serial");
    expect(apiSource).toContain("platform_public_key_pem");
    expect(channelSource).not.toContain("credential_version_id");
    expect(pageSource).toContain("saveChannelCredential");
    expect(pageSource).toContain("revalidate: true");
    expect(channelSource).toContain("clearSensitiveFields");
  });

  test("fails closed in both temporary Channel states after credential replacement", () => {
    const channel = {
      id: "channel-1",
      adapter_kind: "stripe",
      name: "Stripe",
      icon_kind: "builtin",
      icon_value: null,
      sort_order: 0,
      enabled: true,
      revision: 4,
      effective_available: true,
      unavailable_reasons: [],
      supported_currencies: ["CNY", "USD"],
      amount_limits: {
        CNY: { min_minor: "1", max_minor: "10000" },
        USD: { min_minor: "1", max_minor: "10000" },
      },
      checkout_action_kinds: ["redirect"],
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
    } satisfies StorePaymentChannel;

    const disabled = disabledOptimisticChannel(channel);
    expect(disabled).toEqual({
      ...channel,
      enabled: false,
      revision: 5,
      effective_available: false,
      unavailable_reasons: ["channel_disabled"],
      supported_currencies: [],
      amount_limits: {},
      checkout_action_kinds: [],
    });
    expect(channel.effective_available).toBe(true);
    expect(pageSource.match(/disabledOptimisticChannel\(item\)/g)).toHaveLength(2);
  });

  test("removes manual order completion and keeps optimistic rollback", () => {
    expect(pageSource).not.toContain("completeOrder");
    expect(pageSource).not.toContain("cancelOrder");
    expect(pageSource).toContain("optimisticData");
    expect(pageSource).toContain("rollbackOnError: true");
  });

  test("opens an Admin order detail dialog backed by SWR data", () => {
    expect(pageSource).toContain("selectedOrderId");
    expect(pageSource).toContain("getOrderDetail");
    expect(pageSource).toContain("OrderDialog");
    expect(panelsSource).toContain("onSelectOrder");
    expect(orderDialogSource).toContain("rounded-2xl");
    expect(orderDialogSource).toContain("min-h-");
    expect(orderDialogSource).toContain("Skeleton");
    expect(orderDialogSource).toContain("onRetry");
    expect(orderDialogSource).toContain("attempts.map");
    expect(orderDialogSource).toContain("refunds.map");
  });

  test("uses only real Admin order and refund actions with state gating", () => {
    for (const method of [
      "getOrderDetail",
      "queryOrder",
      "closeOrder",
      "createRefund",
      "getRefund",
      "queryRefund",
    ]) {
      expect(apiSource).toContain(method);
    }
    expect(apiSource).toContain('"refund"');
    expect(orderDialogSource).toContain("canQueryAttempt");
    expect(orderDialogSource).toContain("canCloseAttempt");
    expect(orderDialogSource).toContain("canCreateRefund");
    expect(orderDialogSource).toContain('type="password"');
    expect(orderDialogSource).toContain("onCreateRefund");
    expect(orderDialogSource).toContain("onQueryRefund");
    expect(orderDialogSource).not.toContain("Complete");
    expect(orderDialogSource).not.toContain("reprocess");
    expect(orderDialogSource).not.toContain("caseAction");
  });

  test("hides close for an old failed Attempt while another Attempt is active", () => {
    const order = { contract_version: 2, payment_state: "unpaid" } as StoreOrder;
    const failed = {
      id: "attempt-failed",
      adapter_kind: "stripe",
      state: "failed",
      provider_object_id: "pi_failed",
    } as StorePaymentAttempt;
    const active = {
      id: "attempt-active",
      adapter_kind: "stripe",
      state: "presented",
      provider_object_id: "pi_active",
    } as StorePaymentAttempt;

    expect(canCloseAttempt(order, [failed, active], failed)).toBe(false);
    expect(canCloseAttempt(order, [failed], failed)).toBe(true);
  });

  test("revalidates order detail and list after every Admin mutation", () => {
    expect(pageSource).toContain("refreshSelectedOrder");
    expect(pageSource).toContain("orders.mutate()");
    expect(pageSource).toContain("orderDetail.mutate()");
    expect(pageSource).toContain("crypto.randomUUID()");
    expect(pageSource).toContain('createReauthGrant(currentPassword, "refund")');
    expect(pageSource).not.toContain("optimisticRefund");
    expect(pageSource).toContain("mutateSelectedOrderDetail");
    expect(pageSource).not.toMatch(/optimisticData:\s*\(current\)\s*=>\s*current,\s*rollbackOnError/);
    expect(pageSource).toContain("pending_action: actionKey");
    expect(pageSource).toContain("orderDetail.data?.pending_action");
    expect(pageSource).toContain("rollbackOnError: true");
    expect(orderDialogSource).toMatch(/await onQueryRefund\(refundId, currentPassword\);\s*setCurrentPassword\(""\);/);
  });

  test("generates bounded redemption batches and shows plaintext once", () => {
    expect(redemptionSource).toContain('min={1}');
    expect(redemptionSource).toContain('max={20}');
    expect(redemptionSource).toContain('max={365}');
    expect(redemptionSource).toContain("generatedCodes");
    expect(panelsSource).toContain("code_hint");
    expect(pageSource).not.toContain("redemption.code_plaintext");
  });

  test("supports scoped redemption reveal export and revocation", () => {
    expect(apiSource).toContain('scope: "credential_update" | "redemption_access" | "refund"');
    expect(apiSource).toContain("revealRedemptionCodes");
    expect(apiSource).toContain("exportRedemptionCodes");
    expect(apiSource).toContain("revokeRedemptionCode");
    expect(apiSource).toContain('"unused" | "used" | "revoked"');
  });

  test("edits four exact custom amount bounds", () => {
    expect(pageSource).toContain("custom_recharge_cny_min_minor");
    expect(pageSource).toContain("custom_recharge_cny_max_minor");
    expect(pageSource).toContain("custom_recharge_usd_min_minor");
    expect(pageSource).toContain("custom_recharge_usd_max_minor");
    expect(pageSource).toContain("decimalToMinor");
    expect(pageSource).toContain("minorToDecimal");
  });

  test("uses SWR skeletons, retry surfaces, and modal dialogs", () => {
    expect(pageSource).toContain("useSWR");
    expect(panelsSource).toContain("Skeleton");
    expect(panelsSource).toContain("store.admin.retry");
    expect(productSource).toContain("Dialog");
    expect(channelSource).toContain("Dialog");
    expect(redemptionSource).toContain("Dialog");
  });

  test("uses accessible confirmation dialogs for destructive actions", () => {
    expect(pageSource).toContain("AlertDialog");
    expect(pageSource).not.toContain("window.confirm");
  });
});
