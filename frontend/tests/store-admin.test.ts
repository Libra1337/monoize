import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { createInstance } from "i18next";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { StoreAdminTabs } from "../src/pages/store-admin/store-admin-tabs";

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
    expect(channelSource).not.toContain("credential_version_id");
    expect(pageSource).toContain("saveChannelCredential");
    expect(pageSource).toContain("revalidate: true");
    expect(channelSource).toContain("clearSensitiveFields");
  });

  test("removes manual order completion and keeps optimistic rollback", () => {
    expect(pageSource).not.toContain("completeOrder");
    expect(pageSource).not.toContain("cancelOrder");
    expect(pageSource).toContain("optimisticData");
    expect(pageSource).toContain("rollbackOnError: true");
  });

  test("generates bounded redemption batches and shows plaintext once", () => {
    expect(redemptionSource).toContain('min={1}');
    expect(redemptionSource).toContain('max={20}');
    expect(redemptionSource).toContain('max={365}');
    expect(redemptionSource).toContain("generatedCodes");
    expect(panelsSource).toContain("code_hint");
    expect(pageSource).not.toContain("redemption.code_plaintext");
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
