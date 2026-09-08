import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";

const readSource = (relativePath: string) => {
  const url = new URL(relativePath, import.meta.url);
  return existsSync(url) ? readFileSync(url, "utf8") : "";
};

const apiSource = readSource("../src/lib/store-api.ts");
const channelSource = readSource("../src/pages/store-admin/channel-dialog.tsx");
const methodsDialogSource = readSource("../src/pages/store-admin/epay-methods-dialog.tsx");
const adminPageSource = readSource("../src/pages/store-admin/index.tsx");
const panelsSource = readSource("../src/pages/store-admin/admin-panels.tsx");
const governanceStateSource = readSource("../src/pages/store-admin/governance-state.ts");
const storeSource = readSource("../src/pages/store/index.tsx");
const paymentSource = readSource("../src/pages/store/payment-methods.tsx");
const selectionSource = readSource("../src/pages/store/store-selection.ts");
const LOCALES = ["en", "zh", "zh-TW", "ja"] as const;

describe("EPay Store adapter", () => {
  test("permits only epay, stripe, and http adapter kinds", () => {
    expect(apiSource).toContain('export type PaymentAdapterKind = "epay" | "stripe" | "http";');
    expect(apiSource).toContain('export type EpayMethodKind = "alipay" | "wxpay";');
    expect(apiSource).not.toContain('"alipay" | "wechat" | "stripe"');
    expect(channelSource).toContain('["epay", "stripe", "http"]');
  });

  test("builds an EPay credential payload with a normalized gateway and one enabled method", () => {
    expect(channelSource).toContain("gateway_base_url: gateway");
    expect(channelSource).toContain("merchant_key: values[2]");
    expect(channelSource).toContain("alipay_enabled: draft.alipayEnabled");
    expect(channelSource).toContain("wxpay_enabled: draft.wxpayEnabled");
    expect(channelSource).toContain("if (!draft.alipayEnabled && !draft.wxpayEnabled) return null;");
    // The merchant secret is entered as a password field and never prefilled.
    expect(channelSource).toContain('id="store-epay-key"');
    expect(channelSource).toContain('type="password"');
  });

  test("enforces CNY-only readiness metadata for EPay", () => {
    expect(governanceStateSource).toContain('if (adapterKind === "epay")');
    expect(governanceStateSource).toContain('currencies[0] === "CNY"');
    expect(governanceStateSource).not.toContain('adapterKind === "wechat"');
  });

  test("renders one selectable option per enabled EPay method", () => {
    expect(selectionSource).toContain("export function expandPaymentOptions");
    expect(selectionSource).toContain("option.enabled");
    expect(selectionSource).toContain("`${channelId}:${method}`");
    expect(storeSource).toContain("options={paymentOptions}");
    expect(paymentSource).toContain('option.method === "alipay"');
    expect(paymentSource).toContain('option.method === "wxpay"');
  });

  test("sends the exact EPay method as the expected payment method", () => {
    expect(selectionSource).toContain('if (option.channel.adapter_kind === "epay") return option.method;');
    expect(storeSource).toContain("expectedPaymentMethod(validatedOption)");
    expect(storeSource).not.toContain('"computer_web"');
    expect(storeSource).not.toContain('"native"');
  });

  test("exposes an Admin dialog that configures each EPay method", () => {
    expect(apiSource).toContain("putEpayMethod");
    expect(apiSource).toContain("/epay-methods/");
    expect(methodsDialogSource).toContain("storeApi.admin.putEpayMethod");
    expect(methodsDialogSource).toContain("sort_order: sortOrder");
    expect(methodsDialogSource).toContain("enabled,");
    expect(panelsSource).toContain('channel.adapter_kind === "epay"');
    expect(panelsSource).toContain("store.admin.epayMethods.action");
    expect(adminPageSource).toContain("<EpayMethodsDialog");
    expect(adminPageSource).toContain("onSaved={() => channels.mutate()}");
  });

  test("defines EPay copy in every locale", () => {
    for (const locale of LOCALES) {
      const catalog = JSON.parse(readSource(`../src/locales/${locale}.json`));
      const kinds = catalog.store.admin.channels.kinds;
      expect(Object.keys(kinds).sort(), locale).toEqual(["epay", "http", "stripe"]);
      const credential = catalog.store.admin.channels.credential;
      for (const key of [
        "gatewayBaseUrl",
        "merchantKey",
        "alipayEnabled",
        "wxpayEnabled",
        "epayCnyOnly",
      ]) {
        expect(typeof credential[key], `${locale}: credential.${key}`).toBe("string");
      }
      const methods = catalog.store.admin.epayMethods;
      for (const key of [
        "action",
        "title",
        "description",
        "label",
        "sortOrder",
        "enabled",
        "saved",
        "saveFailed",
      ]) {
        expect(typeof methods[key], `${locale}: epayMethods.${key}`).toBe("string");
      }
      expect(typeof methods.methods.alipay, `${locale}: epayMethods.methods.alipay`).toBe("string");
      expect(typeof methods.methods.wxpay, `${locale}: epayMethods.methods.wxpay`).toBe("string");
    }
  });
});
